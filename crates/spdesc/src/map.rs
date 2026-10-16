//! The maps the game can be played on, and where their art comes from.
//!
//! The catalogue is the project's own arena first, then every map `MW2_DATA` ships a zone for. The
//! game's own rotation (`mp/basemaps.arena`, read by `crates/spdesc-mw2`) lists downloadable maps
//! too, so a map the install does not actually have is not offered: the check is the map's own
//! fastfile, because a map's name *is* its fastfile.
//!
//! Each map also carries what the install says about it — the gametypes it offers, its environment,
//! the two factions — and its own preview art, which lives in the `.iwd` archives
//! (`images/preview_<name>.iwi`). A few dozen previews is more than a frame should hold up, so they
//! are read and decoded on their own thread and the handles arrive as they are done.
//!
//! The rule about which map carries the practice targets lives in `spdesc-game-core`; this module
//! only presents it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use spdesc_game_core::{DEV_MAP, GameMap};
use spdesc_mw2::{MapEntry, SoundBank, iwd, iwi};

use crate::combat::{self, Enemy};
use crate::menu::Menu;

/// What the project calls its own arena.
const DEV_TITLE: &str = "Dev arena";

/// One map as the menu lists it: the rule the catalogue holds, and the install's own detail.
pub(crate) struct MapRow {
    pub(crate) map: GameMap,
    /// The archive entry the map's preview art lives at, when the install ships one.
    pub(crate) preview: Option<String>,
    /// The install's own tags for the map: gametypes, environment, factions.
    pub(crate) detail: String,
}

/// The maps on offer: the project's own arena, then the install's, in the game's own order.
pub(crate) fn build_rows(bank: &SoundBank, install: Option<&Path>) -> Vec<MapRow> {
    let mut rows = vec![MapRow {
        map: GameMap {
            id: DEV_MAP.to_owned(),
            title: DEV_TITLE.to_owned(),
            targets: true,
        },
        preview: None,
        detail: "the project's own ground — the practice targets stand here".to_owned(),
    }];
    let Some(install) = install else {
        return rows;
    };
    for entry in bank.maps() {
        if !shipped(install, &entry.id) {
            continue;
        }
        rows.push(MapRow {
            map: GameMap {
                id: entry.id.clone(),
                title: title_of(&entry.id, &entry.title_key),
                targets: false,
            },
            preview: Some(spdesc_mw2::preview_entry(&entry.preview)),
            detail: detail_of(&entry),
        });
    }
    rows
}

/// Whether the install ships a map: its zone is its fastfile under `zone/english`.
fn shipped(install: &Path, id: &str) -> bool {
    spdesc_mw2::map_zone_path(install, id).is_file()
}

/// A map's title from the game's own name key.
///
/// The keys come in more than one shape — `MPUI_RAID` and `PATCH_VACANT` both name a map — so the
/// prefix comes off and the rest is titled a word at a time. A key that names nothing falls back to
/// the map's own name, which is what it is called in the fastfiles.
fn title_of(id: &str, key: &str) -> String {
    let stem = ["MPUI_", "PATCH_", "MP_"]
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .filter(|stem| !stem.is_empty())
        .unwrap_or(id);
    let stem = stem.strip_prefix("mp_").unwrap_or(stem);
    let title = stem
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut letters = word.chars();
            match letters.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>()
                        + &letters.as_str().to_ascii_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        id.to_owned()
    } else {
        title
    }
}

/// The install's own one-line description of what a map is.
fn detail_of(entry: &MapEntry) -> String {
    let mut parts = Vec::new();
    if !entry.gametypes.is_empty() {
        parts.push(entry.gametypes.join(" "));
    }
    if !entry.environment.is_empty() {
        parts.push(entry.environment.clone());
    }
    if !entry.allies.is_empty() || !entry.axis.is_empty() {
        parts.push(format!("{} vs {}", entry.allies, entry.axis));
    }
    parts.join("   ")
}

/// The map previews the install ships, read off the main thread.
#[derive(Resource, Default)]
pub(crate) struct Previews {
    /// One handle per map id, once its art has arrived.
    handles: HashMap<String, Handle<Image>>,
    /// Where the reader sends decoded art. Dropped once it has finished, so the thread ends.
    inbox: Option<Mutex<Receiver<(String, iwi::Image)>>>,
}

impl Previews {
    /// Start reading every preview the catalogue names out of the archives.
    pub fn start(rows: &[MapRow]) -> Previews {
        let mut previews = Previews::default();
        let Some(install) = spdesc_game_core::mw2_data() else {
            return previews;
        };
        let entries: Vec<(String, String)> = rows
            .iter()
            .filter_map(|row| row.preview.clone().map(|entry| (row.map.id.clone(), entry)))
            .collect();
        if entries.is_empty() {
            return previews;
        }
        let (outbox, inbox) = channel();
        previews.inbox = Some(Mutex::new(inbox));
        std::thread::Builder::new()
            .name("spdesc-map-previews".into())
            .spawn(move || {
                // One walk of the archives for all of them: each entry alone would re-open every
                // archive's directory.
                let wanted: Vec<String> = entries.iter().map(|(_, entry)| entry.clone()).collect();
                let found = iwd::read_many(&install, &wanted);
                for (id, entry) in entries {
                    let Some(bytes) = found.get(&entry) else {
                        continue;
                    };
                    let Ok(image) = iwi::decode(bytes) else {
                        continue;
                    };
                    if outbox.send((id, image)).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn the map preview reader");
        previews
    }

    /// The handle for a map's preview, when the reader has finished it.
    pub fn handle(&self, id: &str) -> Option<Handle<Image>> {
        self.handles.get(id).cloned()
    }

    /// Take in what the reader has finished.
    pub fn collect(&mut self, images: &mut Assets<Image>) {
        let mut arrived = Vec::new();
        {
            let Some(inbox) = self.inbox.as_ref().and_then(|inbox| inbox.lock().ok()) else {
                return;
            };
            while let Ok(item) = inbox.try_recv() {
                arrived.push(item);
            }
        }
        for (id, image) in arrived {
            self.handles.insert(id, images.add(to_image(image)));
        }
    }
}

/// A preview as a Bevy image: the art is straight sRGB colour, like the insignia.
fn to_image(image: iwi::Image) -> Image {
    Image::new(
        Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        image.rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Take in the previews the reader has finished.
pub(crate) fn collect_previews(mut previews: ResMut<Previews>, mut images: ResMut<Assets<Image>>) {
    previews.collect(&mut images);
}

/// The map being played, as the gameplay systems need it.
#[derive(Resource, Default)]
pub(crate) struct ChosenMap {
    pub(crate) title: String,
    pub(crate) targets: bool,
}

/// Follow the menu's choice, so the rest of the game knows which map it is on.
pub(crate) fn follow_choice(menu: Res<Menu>, mut chosen: ResMut<ChosenMap>) {
    let (title, targets) = menu.chosen_map();
    if chosen.title != title || chosen.targets != targets {
        info!(
            "map: {title}{}",
            if targets { " (practice targets)" } else { "" }
        );
        chosen.title = title;
        chosen.targets = targets;
    }
}

/// The practice targets belong to the project's own map: they stand there and nowhere else, so
/// changing to an imported map clears them and coming back puts them up again. Nothing is
/// remembered — what stands is read from the world each frame, so it cannot drift.
pub(crate) fn sync_targets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    chosen: Res<ChosenMap>,
    standing: Query<Entity, With<Enemy>>,
) {
    match (chosen.targets, standing.is_empty()) {
        (true, true) => {
            combat::spawn_enemies(&mut commands, &mut meshes, &mut materials, &mut images);
        }
        (false, false) => {
            for entity in &standing {
                commands.entity(entity).despawn();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spdesc_game_core::Maps;

    fn entry(id: &str, key: &str) -> MapEntry {
        MapEntry {
            id: id.to_owned(),
            title_key: key.to_owned(),
            preview: format!("preview_{id}"),
            gametypes: vec!["dm".to_owned(), "dom".to_owned()],
            environment: "urban".to_owned(),
            allies: "us_army".to_owned(),
            axis: "opforce_airborne".to_owned(),
        }
    }

    /// The game's own name keys come in more than one shape, and a key that names nothing falls
    /// back to the map's own name rather than to a made-up one.
    #[test]
    fn a_title_comes_from_the_games_own_name_key() {
        assert_eq!(title_of("mp_alpha", "MPUI_ALPHA"), "Alpha");
        assert_eq!(title_of("mp_beta", "PATCH_BETA"), "Beta");
        assert_eq!(
            title_of("mp_gamma_ridge", "MPUI_GAMMA_RIDGE"),
            "Gamma Ridge"
        );
        assert_eq!(title_of("mp_delta", "MPUI_"), "Delta");
        assert_eq!(title_of("mp_epsilon", ""), "Epsilon");
        assert_eq!(title_of("mp_zeta", "mp_zeta"), "Zeta");
    }

    /// The detail line carries what the install says about the map, and says nothing rather than
    /// something empty when a field is missing.
    #[test]
    fn the_detail_line_says_what_the_install_says() {
        let full = detail_of(&entry("mp_alpha", "MPUI_ALPHA"));
        assert_eq!(full, "dm dom   urban   us_army vs opforce_airborne");
        let mut bare = entry("mp_beta", "MPUI_BETA");
        bare.gametypes.clear();
        bare.environment.clear();
        bare.allies.clear();
        bare.axis.clear();
        assert_eq!(detail_of(&bare), "");
    }

    /// The catalogue always opens with the project's own map, which is the one with the targets, and
    /// the rule the choices are resolved against agrees with the rows it was built from.
    #[test]
    fn the_catalogue_opens_with_the_projects_own_map() {
        let rows = build_rows(&SoundBank::default(), None);
        assert_eq!(rows.len(), 1, "no install, so only the project's own map");
        assert_eq!(rows[0].map.id, DEV_MAP);
        assert!(rows[0].map.targets);
        assert!(rows[0].preview.is_none(), "the arena has no shipped art");

        let maps = Maps::new(
            rows[0].map.clone(),
            rows.iter().skip(1).map(|row| row.map.clone()),
        );
        assert_eq!(maps.default_id(), Some(DEV_MAP));
        assert_eq!(maps.len(), rows.len());
    }

    /// Every preview the install ships arrives, decodes, and is a picture: this is the whole path
    /// the menu depends on — the archives, the `.iwi` decoder, and the Bevy image it shows.
    #[test]
    fn every_maps_preview_arrives_and_decodes() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let bank = SoundBank::read(&install, &["code_post_gfx_mp"]);
        let rows = build_rows(&bank, Some(&install));
        let wanted = rows.iter().filter(|row| row.preview.is_some()).count();
        assert!(wanted > 10, "only {wanted} maps ship a preview");

        let mut previews = Previews::start(&rows);
        let mut images = Assets::<Image>::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            previews.collect(&mut images);
            let have = rows
                .iter()
                .filter(|row| previews.handle(&row.map.id).is_some())
                .count();
            if have == wanted {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {have} of {wanted} previews arrived"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        for row in rows.iter().filter(|row| row.preview.is_some()) {
            let handle = previews.handle(&row.map.id).expect("all arrived above");
            let image = images.get(&handle).expect("the image was added");
            let size = image.size();
            assert!(
                size.x > 100 && size.y > 100,
                "{}: a preview of {size}",
                row.map.title
            );
            let data = image.data.as_ref().expect("the pixels are kept");
            let shades: std::collections::HashSet<&[u8; 4]> =
                data.as_chunks::<4>().0.iter().collect();
            assert!(
                shades.len() > 64,
                "{}: {} distinct colours is not art",
                row.map.title,
                shades.len()
            );
        }
    }

    /// Every map the install's own rotation lists either ships a zone — and is offered, with its
    /// art — or does not, and is left out rather than offered as a map that cannot be played.
    #[test]
    fn only_maps_the_install_ships_are_offered() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let bank = SoundBank::read(&install, &["code_post_gfx_mp"]);
        let listed = bank.maps();
        let rows = build_rows(&bank, Some(&install));
        assert_eq!(
            rows.len(),
            listed
                .iter()
                .filter(|entry| shipped(&install, &entry.id))
                .count()
                + 1,
            "one row per shipped map, plus the project's own"
        );
        for row in rows.iter().skip(1) {
            assert!(shipped(&install, &row.map.id));
            assert!(
                !row.map.targets,
                "an imported map carries no practice targets"
            );
            assert!(!row.map.title.is_empty());
            assert!(row.preview.is_some());
            assert!(
                row.map.id.starts_with("mp_"),
                "{} is not a map name",
                row.map.id
            );
        }
    }
}
