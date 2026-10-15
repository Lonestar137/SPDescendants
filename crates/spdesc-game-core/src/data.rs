//! Where the external data folders live: one resolver per read-only input.
//!
//! Two roots are searched the same way, and the first candidate that exists wins:
//!
//! 1. `$<NAME>`. Set and empty (`TFD_DATA=`) means no data and ends the search, so the no-data
//!    path runs on a machine that has data.
//! 2. `<this checkout>/<NAME>`, the dev link, `dev` builds only ([`project_folder`]).
//! 3. `<exe dir>/<NAME>`, then `<exe dir>/data` where the root claims it, the shipped layouts.
//!
//! - [`TFD`] — `TFD_DATA`, The First Descendant. The meshes, textures and animation the game is
//!   built from.
//! - [`MW2`] — `MW2_DATA`, Call of Duty: Modern Warfare 2 (2009). Optional: the sound effects are
//!   read from the fastfiles it ships (see `docs/research/MW2-AUDIO.md`).
//!
//! Either folder is read-only runtime input: the resolvers and everything that consumes them may
//! read a folder and must never write into it, and nothing under either root is ever committed
//! (see the project's `docs/METHOD.md`).

use std::path::{Path, PathBuf};

/// A read-only external data root: the folder name, which the environment variable that overrides
/// it is spelled from, and the switch that turns a data-gated test's skip into a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DataRoot {
    /// The folder name, beside the checkout and the binary, and the variable that overrides it.
    pub name: &'static str,
    /// The variable that turns a data-gated test's skip into a failure once this root resolves.
    require_env: &'static str,
    /// Whether `<exe dir>/data` is this root's shipped fallback.
    ///
    /// Only `TFD_DATA` names it: `data` is the older shipped spelling of the game's own folder,
    /// and a second root must not claim it.
    data_dir_fallback: bool,
}

/// The First Descendant's install root: meshes, textures, animation.
pub const TFD: DataRoot = DataRoot {
    name: "TFD_DATA",
    require_env: "SPDESC_REQUIRE_DATA",
    data_dir_fallback: true,
};

/// Call of Duty: Modern Warfare 2 (2009): the sound effects.
pub const MW2: DataRoot = DataRoot {
    name: "MW2_DATA",
    require_env: "SPDESC_REQUIRE_MW2",
    data_dir_fallback: false,
};

/// Both roots, for a caller that has to sweep every folder the app reads.
pub const ALL: [DataRoot; 2] = [TFD, MW2];

impl DataRoot {
    /// Every place this root is looked for, in order, whether or not it exists: the "looked in"
    /// message of a skipped test, and the search [`resolve`](Self::resolve) runs.
    pub fn candidates(&self) -> Vec<PathBuf> {
        self.candidates_from(
            std::env::var_os(self.name).map(PathBuf::from),
            project_folder(),
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(Path::to_path_buf)),
        )
    }

    /// [`candidates`](Self::candidates) with the environment passed in: a test that set `$TFD_DATA`
    /// would change the answer for every test running beside it. The `dev` fallback is passed in
    /// too, so the pure search is testable without feature flags.
    fn candidates_from(
        &self,
        override_dir: Option<PathBuf>,
        project: Option<&Path>,
        exe_dir: Option<PathBuf>,
    ) -> Vec<PathBuf> {
        let mut out = Vec::with_capacity(4);

        // 1. The override; set and empty is no data at all, even in a dev build
        // (`TFD_DATA= cargo test` runs the no-data tests on a machine that has data).
        if let Some(over) = override_dir {
            if over.as_os_str().is_empty() {
                return Vec::new();
            }
            out.push(over);
        }

        // 2. The checkout's link, dev builds only.
        if let Some(root) = project {
            out.push(root.join(self.name));
        }

        // 3. Beside the binary; a failed `current_exe` adds nothing.
        if let Some(dir) = exe_dir {
            out.push(dir.join(self.name));
            if self.data_dir_fallback {
                out.push(dir.join("data"));
            }
        }

        out
    }

    /// This root's folder, or `None` when there is none. Uncached, so an environment change is
    /// seen.
    pub fn resolve(&self) -> Option<PathBuf> {
        self.candidates().into_iter().find(|c| c.is_dir())
    }

    /// A data-gated test's skip when this root does not resolve, naming every path tried. Under
    /// [`require_env`](Self::require_env) it fails instead, which the gate sets where this root
    /// resolves.
    pub fn skip(&self, what: &str) {
        self.skip_looked_in(what, &self.candidates());
    }

    /// [`skip`](Self::skip) with the searched paths passed in, for a check whose search is not
    /// this root's own candidates.
    pub fn skip_looked_in(&self, what: &str, looked_in: &[PathBuf]) {
        skipped_under(
            required(self.require_env),
            self.require_env,
            what,
            looked_in,
        );
        log_skip(what);
    }
}

/// The First Descendant's install root, or `None` when it is not there.
pub fn tfd_data() -> Option<PathBuf> {
    TFD.resolve()
}

/// Modern Warfare 2's install root, or `None` when it is not there.
pub fn mw2_data() -> Option<PathBuf> {
    MW2.resolve()
}

/// This crate's manifest dir, two levels up: the checkout a `dev` build keeps its data links in
/// (gitignored; they point outside the checkout).
/// `None` in a build that turned the default `dev` feature off, which looks in no source tree.
#[cfg(feature = "dev")]
fn project_folder() -> Option<&'static Path> {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)
}

#[cfg(not(feature = "dev"))]
fn project_folder() -> Option<&'static Path> {
    None
}

/// A data-gated test's skip: `tfd_data_or_skip!()` in a `-> ()` test, `tfd_data_or_skip!(None)` in
/// a helper returning `Option`. The skip names every path tried, and fails instead once
/// `SPDESC_REQUIRE_DATA` is set.
#[macro_export]
macro_rules! tfd_data_or_skip {
    () => {
        $crate::tfd_data_or_skip!(())
    };
    ($ret:expr) => {
        match $crate::tfd_data() {
            Some(data) => data,
            None => {
                $crate::TFD.skip("no TFD_DATA folder found");
                return $ret;
            }
        }
    };
}

/// [`tfd_data_or_skip!`] for the Modern Warfare 2 root, gated by `SPDESC_REQUIRE_MW2`.
#[macro_export]
macro_rules! mw2_data_or_skip {
    () => {
        $crate::mw2_data_or_skip!(())
    };
    ($ret:expr) => {
        match $crate::mw2_data() {
            Some(data) => data,
            None => {
                $crate::MW2.skip("no MW2_DATA folder found");
                return $ret;
            }
        }
    };
}

/// A data-gated test's skip, printed; a panic under the root's require switch, which the gate sets
/// where that data resolves, since libtest hides a passing test's stderr and a lost data read goes
/// green.
#[doc(hidden)]
pub fn skipped(require_env: &str, what: &str, looked_in: &[PathBuf]) {
    skipped_under(required(require_env), require_env, what, looked_in);
    log_skip(what);
}

/// Whether a require switch is on. `SPDESC_REQUIRE_DATA=0` is off; any other set value is on.
fn required(require_env: &str) -> bool {
    std::env::var_os(require_env).is_some_and(|v| !v.is_empty() && v != "0")
}

/// `$SPDESC_SKIP_LOG`: libtest hides a passing test's stderr, so the gate counts skips from this
/// file instead.
fn log_skip(what: &str) {
    if let Some(log) = std::env::var_os("SPDESC_SKIP_LOG").filter(|p| !p.is_empty()) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
        {
            // One write per line: parallel tests append here, and a formatted write can interleave.
            let _ = f.write_all(format!("{what}\n").as_bytes());
        }
    }
}

/// [`skipped`] with the switch passed in, testable without the process environment.
fn skipped_under(required: bool, require_env: &str, what: &str, looked_in: &[PathBuf]) {
    let msg = format!("skipping: {what} — looked in {looked_in:?}");
    assert!(
        !required,
        "{msg} — and {require_env} is set: the gate resolved this data on this machine, so a \
         skip here is a broken resolver or a missing folder, not missing data"
    );
    eprintln!("{msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_empty_override_ends_the_search() {
        assert_eq!(
            TFD.candidates_from(Some(PathBuf::new()), Some(Path::new("/repo")), None),
            Vec::<PathBuf>::new(),
            "TFD_DATA= (set, empty) is 'no data', not a fallthrough"
        );
    }

    #[test]
    fn an_override_is_first_and_the_fallbacks_still_follow() {
        // A mistyped `TFD_DATA=` path falls through rather than dead-ending, so the search
        // stays one list the "looked in" message can print — the override is first, not alone.
        let got = TFD.candidates_from(
            Some(PathBuf::from("/mnt/data")),
            Some(Path::new("/repo")),
            None,
        );
        assert_eq!(
            got,
            vec![
                PathBuf::from("/mnt/data"),
                Path::new("/repo").join("TFD_DATA")
            ]
        );
    }

    #[test]
    fn without_an_override_the_checkout_then_the_binary_layout_are_tried_in_order() {
        let got = TFD.candidates_from(
            None,
            Some(Path::new("/repo")),
            Some(PathBuf::from("/opt/spdesc")),
        );
        assert_eq!(
            got,
            vec![
                Path::new("/repo").join("TFD_DATA"),
                Path::new("/opt/spdesc").join("TFD_DATA"),
                Path::new("/opt/spdesc").join("data"),
            ]
        );
    }

    /// Each root is looked for under its own name, and only `TFD_DATA` also answers to the older
    /// shipped `data` folder, so a shipped build cannot mistake one root for the other.
    #[test]
    fn each_root_is_named_and_only_the_game_root_claims_the_data_folder() {
        let exe = Some(PathBuf::from("/opt/spdesc"));
        let mw2 = MW2.candidates_from(None, Some(Path::new("/repo")), exe.clone());
        assert_eq!(
            mw2,
            vec![
                Path::new("/repo").join("MW2_DATA"),
                Path::new("/opt/spdesc").join("MW2_DATA"),
            ]
        );
        assert!(
            !mw2.contains(&Path::new("/opt/spdesc").join("data")),
            "a second root must not claim the game's `data` folder"
        );
        assert_ne!(TFD.name, MW2.name, "the roots read different variables");
        assert_ne!(
            TFD.require_env, MW2.require_env,
            "each root's skip fails on its own switch"
        );
    }

    #[test]
    fn no_candidates_at_all_is_a_skip_not_a_panic() {
        skipped_under(false, TFD.require_env, "no TFD_DATA folder found", &[]);
    }

    #[test]
    #[should_panic(expected = "SPDESC_REQUIRE_DATA is set")]
    fn required_data_refuses_to_skip() {
        skipped_under(
            true,
            TFD.require_env,
            "no TFD_DATA folder found",
            &[PathBuf::from("/nowhere")],
        );
    }

    /// The MW2 root fails on its own switch, so a machine with the game's data but no MW2 install
    /// still reports an honest skip rather than a failure.
    #[test]
    #[should_panic(expected = "SPDESC_REQUIRE_MW2 is set")]
    fn the_sound_root_refuses_to_skip_on_its_own_switch() {
        skipped_under(
            true,
            MW2.require_env,
            "no MW2_DATA folder found",
            &[PathBuf::from("/nowhere")],
        );
    }

    #[test]
    fn the_dev_fallback_points_at_this_checkout() {
        #[cfg(feature = "dev")]
        assert_eq!(
            project_folder(),
            Some(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .ancestors()
                    .nth(2)
                    .unwrap()
            ),
            "the dev fallback is the checkout, two levels above this manifest"
        );
        #[cfg(not(feature = "dev"))]
        assert_eq!(
            project_folder(),
            None,
            "a shipped build looks in no source tree"
        );
    }
}
