//! Building character models off the main thread.
//!
//! A model is a body mesh, a head mesh, a face bust, their decoded textures, and the animation
//! clips — long enough to notice, and the menu wants to *preview*, which means putting the
//! highlighted choice on the character without a hitch. So the work runs on a worker thread that
//! owns one opened container. The app says which model it wants and prefetches the ones a step
//! away; the worker builds them oldest-first and hands them back through a channel to be cached.
//!
//! The main thread never parses: it asks, and it spends. The cache is deliberately small — a built
//! model holds its textures at full size, so keeping many would cost more memory than the hitch it
//! saves. Three is enough that stepping back and forth is instant.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use anyhow::Result;
use bevy::prelude::*;

use crate::content::{self, CharacterAssets, DataSource};

/// How many built models are kept. A built model holds its decoded textures, so this trades memory
/// for the hitch of rebuilding: three is enough to step back and forth without waiting.
const CACHE: usize = 3;

/// How many prefetches may be in flight at once. A burst of steps must not queue the whole catalog,
/// and the model being shown is always asked for outside this budget.
const PREFETCH: usize = 2;

/// Everything that decides a model's bytes. Two equal keys produce the same model, which is what
/// makes the cache and the prefetch worth anything.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ModelKey {
    /// The body mesh package path.
    pub body: String,
    /// The head mesh package path (the skin's default head, or the chosen head variant).
    pub head: String,
    /// Folder holding the body skin's `Texture*.uasset` files.
    pub body_dir: String,
    /// Folder holding the head's textures.
    pub head_dir: String,
    /// Folder holding the face (makeup) preset's textures, when one is chosen.
    pub face_dir: Option<String>,
}

impl ModelKey {
    /// Build this model from an already-opened container. The worker owns the container across
    /// requests, so this is all it runs per model.
    pub fn load(&self, data: &mut DataSource) -> Result<CharacterAssets> {
        content::load_character(
            data,
            &self.body,
            &self.head,
            &self.body_dir,
            &self.head_dir,
            self.face_dir.as_deref(),
        )
    }

    /// A short name for logs: enough to tell two models apart without the whole path.
    pub fn label(&self) -> String {
        let stem = |path: &str| {
            path.rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".uasset")
                .to_string()
        };
        format!("{} / {}", stem(&self.body), stem(&self.head))
    }
}

/// A build the worker has finished: the key that was asked for, and either the model or why it
/// could not be made.
enum Done {
    /// The model, boxed so a `Done` stays the size of its key rather than of a whole character.
    Built {
        key: ModelKey,
        assets: Result<Box<CharacterAssets>, String>,
    },
    /// The request was overtaken by a newer one before it started, so nothing was built for it. It
    /// is not a failure: the key simply becomes askable again.
    Superseded(ModelKey),
}

/// Models built off the main thread, and the few kept ready to show.
#[derive(Resource)]
pub struct ModelLoader {
    outbox: Sender<ModelKey>,
    /// What the worker has finished. A `Mutex` because a Bevy resource must be `Sync` and a channel
    /// receiver is not.
    inbox: Mutex<Receiver<Done>>,
    /// Keys asked for and not yet back, so one is never asked for twice.
    waiting: HashSet<ModelKey>,
    /// Keys that failed to build. Asking again would only fail again, and the worker would spin.
    failed: HashSet<ModelKey>,
    /// Built models, oldest first.
    built: HashMap<ModelKey, CharacterAssets>,
    order: VecDeque<ModelKey>,
}

impl ModelLoader {
    /// Start the worker thread and hand back the handle the app talks to.
    pub fn start() -> ModelLoader {
        let (outbox, worker_in) = channel();
        let (worker_out, inbox) = channel();
        std::thread::Builder::new()
            .name("spdesc-model-loader".into())
            .spawn(move || worker(worker_in, worker_out))
            .expect("spawn the model worker thread");
        ModelLoader {
            outbox,
            inbox: Mutex::new(inbox),
            waiting: HashSet::new(),
            failed: HashSet::new(),
            built: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Ask the worker for `key`, unless it is already built, on its way, or known to fail. A
    /// `prefetch` is refused while [`PREFETCH`] builds are already queued. Answers whether the ask
    /// went out.
    pub fn request(&mut self, key: ModelKey, prefetch: bool) -> bool {
        if self.built.contains_key(&key)
            || self.waiting.contains(&key)
            || self.failed.contains(&key)
        {
            return false;
        }
        if prefetch && self.waiting.len() >= PREFETCH {
            return false;
        }
        // A send only fails if the worker is gone, and there is nothing to do about that here: the
        // model simply never arrives and the character keeps the one it has.
        let _ = self.outbox.send(key.clone());
        info!("model: building {} (prefetch {prefetch})", key.label());
        self.waiting.insert(key);
        true
    }

    /// Take the built model for `key`, if the worker has finished it. Whoever spawns it consumes
    /// it, so it leaves the cache.
    pub fn take(&mut self, key: &ModelKey) -> Option<CharacterAssets> {
        self.collect();
        let assets = self.built.remove(key)?;
        self.order.retain(|held| held != key);
        Some(assets)
    }

    /// Take in everything the worker has finished, dropping the oldest models beyond [`CACHE`].
    pub fn collect(&mut self) {
        let mut arrived: Vec<Done> = Vec::new();
        {
            // The guard is dropped before anything is stored, so the receiver's borrow ends with
            // the drain.
            let Ok(inbox) = self.inbox.lock() else {
                return;
            };
            while let Ok(done) = inbox.try_recv() {
                arrived.push(done);
            }
        }
        for done in arrived {
            match done {
                Done::Superseded(key) => {
                    self.waiting.remove(&key);
                }
                Done::Built { key, assets } => {
                    self.waiting.remove(&key);
                    match assets {
                        Ok(assets) => {
                            if self.built.insert(key.clone(), *assets).is_none() {
                                self.order.push_back(key);
                            }
                            self.trim();
                        }
                        Err(error) => {
                            warn!("model: could not build {}: {error}", key.label());
                            self.failed.insert(key);
                        }
                    }
                }
            }
        }
    }

    /// Drop the oldest built models past [`CACHE`]. A key that is still wanted is asked for again
    /// if it is needed, so an eviction only costs a rebuild.
    fn trim(&mut self) {
        while self.order.len() > CACHE {
            if let Some(oldest) = self.order.pop_front() {
                self.built.remove(&oldest);
            }
        }
    }

    /// A loader with no worker behind it: the bookkeeping on its own, for tests. The returned
    /// receiver keeps the request channel open.
    #[cfg(test)]
    fn inert() -> (ModelLoader, Receiver<ModelKey>) {
        let (outbox, held) = channel();
        let (_worker_out, inbox) = channel();
        (
            ModelLoader {
                outbox,
                inbox: Mutex::new(inbox),
                waiting: HashSet::new(),
                failed: HashSet::new(),
                built: HashMap::new(),
                order: VecDeque::new(),
            },
            held,
        )
    }
}

/// The newest of a burst of requests, and the ones it overtook. The worker asks for the newest and
/// reports the rest as superseded, so a burst of steps costs one build rather than one each.
fn newest<T>(first: T, rest: impl Iterator<Item = T>) -> (T, Vec<T>) {
    let mut latest = first;
    let mut passed = Vec::new();
    for newer in rest {
        passed.push(std::mem::replace(&mut latest, newer));
    }
    (latest, passed)
}

/// The worker: open the container once, then build what it is asked for.
fn worker(requests: Receiver<ModelKey>, done: Sender<Done>) {
    let mut data: Option<DataSource> = None;
    while let Ok(first) = requests.recv() {
        let (key, passed) = newest(first, std::iter::from_fn(|| requests.try_recv().ok()));
        for old in passed {
            if done.send(Done::Superseded(old)).is_err() {
                return;
            }
        }
        if data.is_none() {
            match content::open_data() {
                Ok(source) => data = Some(source),
                Err(error) => {
                    let _ = done.send(Done::Built {
                        key,
                        assets: Err(format!("{error:#}")),
                    });
                    continue;
                }
            }
        }
        let assets = key
            .load(data.as_mut().expect("opened above"))
            .map(Box::new)
            .map_err(|error| format!("{error:#}"));
        if done.send(Done::Built { key, assets }).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(body: &str) -> ModelKey {
        ModelKey {
            body: body.to_string(),
            head: "head".to_string(),
            body_dir: "body".to_string(),
            head_dir: "head".to_string(),
            face_dir: None,
        }
    }

    /// A key is asked for once while it is on its way: asking again does nothing, which is what
    /// keeps a highlighted model from being requested every frame it is on screen.
    #[test]
    fn a_key_on_its_way_is_not_asked_for_again() {
        let (mut loader, _held) = ModelLoader::inert();
        assert!(loader.request(key("a"), false), "the first ask goes out");
        assert!(
            !loader.request(key("a"), false),
            "a key already on its way is not asked for twice"
        );
    }

    /// A prefetch is refused once enough work is queued, so a burst of steps cannot queue the whole
    /// catalog; the model being shown is not subject to that budget.
    #[test]
    fn prefetches_are_capped_but_the_wanted_model_is_not() {
        let (mut loader, _held) = ModelLoader::inert();
        for i in 0..PREFETCH {
            assert!(loader.request(key(&format!("p{i}")), true), "prefetch {i}");
        }
        assert!(
            !loader.request(key("late"), true),
            "the prefetch budget is full"
        );
        assert!(
            loader.request(key("wanted"), false),
            "the model being shown is always asked for"
        );
    }

    /// A model that could not be built is not asked for again: it would fail the same way, and the
    /// worker would spend itself on it.
    #[test]
    fn a_model_that_failed_is_not_asked_for_again() {
        let (mut loader, _held) = ModelLoader::inert();
        loader.failed.insert(key("broken"));
        assert!(!loader.request(key("broken"), false));
        assert!(!loader.request(key("broken"), true));
    }

    /// A burst of requests collapses to the newest: the worker builds the choice the player landed
    /// on and reports the ones it passed, rather than building each one in turn.
    #[test]
    fn a_burst_of_requests_collapses_to_the_newest() {
        let (latest, passed) = newest(key("first"), [key("second"), key("third")].into_iter());
        assert_eq!(latest.body, "third");
        assert_eq!(
            passed
                .iter()
                .map(|key| key.body.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"],
            "everything overtaken is reported, oldest first"
        );

        let (latest, passed) = newest(key("only"), std::iter::empty());
        assert_eq!(latest.body, "only");
        assert!(passed.is_empty(), "nothing was overtaken");
    }

    /// At most [`CACHE`] built models are kept, and the ones dropped are the oldest, so the models
    /// just stepped through are the ones that stay.
    #[test]
    fn only_the_newest_models_are_kept() {
        let (mut loader, _held) = ModelLoader::inert();
        for i in 0..CACHE + 2 {
            loader.order.push_back(key(&format!("m{i}")));
        }
        loader.trim();
        let kept: Vec<String> = loader.order.iter().map(|key| key.body.clone()).collect();
        assert_eq!(kept.len(), CACHE);
        assert_eq!(
            kept,
            ["m2", "m3", "m4"],
            "the oldest two make room for the newest"
        );
    }
}
