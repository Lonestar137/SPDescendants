//! Reading Modern Warfare 2's shipped sound assets.
//!
//! The sound effects are not in the loose files: the weapon, footstep and hitmarker cues live
//! inside the fastfiles under `<MW2_DATA>/zone/english`, so reaching them means walking a zone the
//! way the game does. That container work is vendored from IW4L (`vendor/iw4/`); this crate is the
//! project's own reader on top of it — inflate a zone ([`open_zone`]), walk it for its sound
//! aliases and audio ([`read_zone`]), and look a name up ([`SoundBank`]).
//!
//! Nothing here writes: a fastfile is opened read-only and its payloads are copied out. The audio
//! stays in memory or in the player's local state — never in the repository (`docs/METHOD.md`).

pub mod iwd;
pub mod iwi;
pub mod maps;
mod open;
mod sounds;
pub mod world;

use std::path::{Path, PathBuf};

pub use maps::{MapEntry, map_zone_path, parse_arena, preview_entry};
pub use open::{Error, Memory, ZoneImage, open_zone, parse_zone_image};
pub use sounds::{LoadedSound, PCM, RawFile, SoundAlias, Table, ZoneData, read_zone};
pub use world::{
    ClipGeometry, MapReport, MaterialCounts, SurfaceRun, WorldCounts, read_clip, read_map,
    read_runs,
};

/// The zones a Modern Warfare 2 install keeps its fastfiles in, under the install root.
pub const ZONE_DIR: &str = "zone/english";

/// The zones the app reads, in order.
///
/// - `common` is the always-loaded zone: the weapon, foley and impact sounds the game needs on
///   every map, and the level-up cue's streamed file.
/// - `code_post_gfx` holds the interface cues, including the prestige ones.
/// - `code_post_gfx_mp` is small (188 KB) and holds the multiplayer tables: `mp/ranktable.csv`,
///   `mp/rankicontable.csv`, `mp/unlocktable.csv`.
pub const ZONES: [&str; 3] = ["common", "code_post_gfx", "code_post_gfx_mp"];

/// The fastfile `zone` in the install at `install`, whether or not it exists.
pub fn zone_path(install: &Path, zone: &str) -> PathBuf {
    install.join(ZONE_DIR).join(format!("{zone}.ff"))
}

/// Every alias the named zones hold, with the payload of each.
///
/// A zone that will not open, or whose walk stops part-way, contributes what it read; the whole
/// install is optional, so a missing folder is an empty bank rather than an error.
#[derive(Default)]
pub struct SoundBank {
    install: PathBuf,
    zones: Vec<(String, ZoneData)>,
    failures: Vec<(String, Error)>,
}

impl SoundBank {
    /// Read `zones` from the install at `install`.
    pub fn read(install: &Path, zones: &[&str]) -> SoundBank {
        let mut bank = SoundBank {
            install: install.to_path_buf(),
            ..SoundBank::default()
        };
        for zone in zones {
            let path = zone_path(install, zone);
            if !path.is_file() {
                continue;
            }
            match open_zone(&path).and_then(|image| read_zone(&image)) {
                Ok(set) => bank.zones.push(((*zone).to_owned(), set)),
                Err(error) => bank.failures.push(((*zone).to_owned(), error)),
            }
        }
        bank
    }

    /// The zone walks, for a caller that wants to report what was read.
    pub fn zones(&self) -> &[(String, ZoneData)] {
        &self.zones
    }

    /// The zones that could not be read, with why.
    pub fn failures(&self) -> &[(String, Error)] {
        &self.failures
    }

    /// How many aliases every zone contributed together.
    pub fn alias_count(&self) -> usize {
        self.zones.iter().map(|(_, set)| set.aliases.len()).sum()
    }

    /// The first alias named exactly `name`, and its payload when it is a loaded sound.
    pub fn find(&self, name: &str) -> Option<(&SoundAlias, Option<&LoadedSound>)> {
        self.zones.iter().find_map(|(_, set)| {
            set.aliases
                .iter()
                .find(|a| a.name == name)
                .map(|a| (a, set.payload(a)))
        })
    }

    /// The payload of the first alias named exactly `name`.
    pub fn payload(&self, name: &str) -> Option<&LoadedSound> {
        self.find(name)?.1
    }

    /// A cue's audio as a WAV, whether the alias is loaded or streamed.
    ///
    /// A *loaded* alias holds its audio inside the zone, and [`LoadedSound::to_wav`] wraps it. A
    /// *streamed* one names a file in the install's `.iwd` archives, which already is a WAV —
    /// the level-up cue is one of those.
    pub fn audio_wav(&self, name: &str) -> Option<Vec<u8>> {
        let (alias, payload) = self.find(name)?;
        if let Some(payload) = payload
            && let Some(wav) = payload.to_wav()
        {
            return Some(wav);
        }
        let (directory, file) = alias.streamed.as_ref()?;
        let entry = iwd::streamed_sound_entry(directory, file);
        let bytes = iwd::read_entry(&self.install, &entry)?;
        bytes.starts_with(b"RIFF").then_some(bytes)
    }

    /// The payload of the first alias whose name `predicate` accepts, lowest name first so the
    /// choice is stable across runs.
    pub fn payload_matching(
        &self,
        predicate: impl Fn(&str) -> bool,
    ) -> Option<(&SoundAlias, &LoadedSound)> {
        let mut best: Option<(&SoundAlias, &LoadedSound)> = None;
        for (_, set) in &self.zones {
            for alias in &set.aliases {
                if !predicate(alias.name.as_str()) {
                    continue;
                }
                let Some(payload) = set.payload(alias) else {
                    continue;
                };
                if best.is_none_or(|(b, _)| alias.name < b.name) {
                    best = Some((alias, payload));
                }
            }
        }
        best
    }

    /// The multiplayer maps the zones carry, in the order the game's own arena file states them.
    ///
    /// An empty list means no zone held `mp/basemaps.arena`, which is the same as an install that
    /// has no maps to offer.
    pub fn maps(&self) -> Vec<MapEntry> {
        maps::read_maps(&self.zones)
    }

    /// Every alias name in every zone, sorted and deduplicated.
    pub fn alias_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .zones
            .iter()
            .flat_map(|(_, set)| set.aliases.iter().map(|a| a.name.as_str()))
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }
}

impl std::fmt::Debug for SoundBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundBank")
            .field("zones", &self.zones.len())
            .field("aliases", &self.alias_count())
            .field("failures", &self.failures.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The level-up cue is a *streamed* alias: its audio is a WAV in the `.iwd` archives rather
    /// than payload inside the zone, so this checks the alias, the path it names, and the file.
    #[test]
    fn the_level_up_cue_reads_out_of_the_archives() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let bank = SoundBank::read(&install, &["common"]);
        let (alias, _) = bank
            .find("sp_level_up")
            .expect("`sp_level_up` is in common.ff");
        let (dir, file) = alias
            .streamed
            .clone()
            .expect("the level-up cue is streamed, not loaded");
        let entry = iwd::streamed_sound_entry(&dir, &file);
        let bytes =
            iwd::read_entry(&install, &entry).expect("the streamed file is in the archives");
        assert!(
            bytes.len() > 100_000,
            "the level-up cue is {} bytes",
            bytes.len()
        );
        assert_eq!(&bytes[0..4], b"RIFF", "{entry} is not a WAV");
        assert_eq!(&bytes[8..12], b"WAVE", "{entry} is not a WAV");
    }

    /// Every icon the rank tables name decodes to a 64x64 RGBA image with real content, so the
    /// mapping from a table value to a file in the archives cannot drift from the art.
    #[test]
    fn the_rank_and_prestige_icons_decode() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let bank = SoundBank::read(&install, &["code_post_gfx_mp"]);
        let Some(zone) = bank.zones().first().map(|(_, data)| data) else {
            panic!("code_post_gfx_mp did not open: {:?}", bank.failures());
        };
        let icons = zone
            .table("mp/rankicontable.csv")
            .expect("the rank icon table is in code_post_gfx_mp");
        assert!(icons.rows >= 71, "one row per rank, plus the header");

        // Every name the table uses for a rank resolves to art in the archives: this is the check
        // that catches a misspelled mapping, and it covers all 22 insignia rather than a sample.
        let mut named: Vec<&str> = (0..70).filter_map(|r| icons.cell(r + 2, 1)).collect();
        named.sort_unstable();
        named.dedup();
        assert_eq!(named.len(), 22, "the rank table names 22 insignia");
        for name in &named {
            let entry = iwd::rank_icon_entry(name);
            assert!(
                iwd::read_entry(&install, &entry).is_some(),
                "{name} maps to {entry}, which is not in the archives"
            );
        }

        let mut checked = 0;
        // Row 0 is the header, row 1 is `maxprestige`, so rank `r` is row `r + 2`, whose first
        // column is the rank index and whose second is its icon.
        for rank in [0usize, 3, 17, 45, 69] {
            assert_eq!(
                icons.cell(rank + 2, 0),
                Some(rank.to_string().as_str()),
                "the rank table is indexed by rank"
            );
            let name = icons.cell(rank + 2, 1).expect("a rank icon name");
            let image = read_icon(&install, &iwd::rank_icon_entry(name));
            assert_eq!(
                ((image.width, image.height), image.rgba.len()),
                ((64, 64), 64 * 64 * 4)
            );
            assert!(
                has_content(&image.rgba),
                "{name} decoded empty: {}",
                describe(&image)
            );
            checked += 1;
        }
        for prestige in 1..=10u32 {
            let image = read_icon(&install, &iwd::prestige_icon_entry(prestige));
            assert!(
                has_content(&image.rgba),
                "prestige {prestige} decoded empty"
            );
            checked += 1;
        }
        assert_eq!(checked, 15);
    }

    fn read_icon(install: &std::path::Path, entry: &str) -> iwi::Image {
        let bytes =
            iwd::read_entry(install, entry).unwrap_or_else(|| panic!("{entry} is in the archives"));
        iwi::decode(&bytes).unwrap_or_else(|error| panic!("{entry}: {error}"))
    }

    fn describe(image: &iwi::Image) -> String {
        let texels = image.rgba.len() / 4;
        let mut opaque = 0;
        let mut clear = 0;
        let mut shades = std::collections::HashSet::new();
        for t in image.rgba.as_chunks::<4>().0 {
            if t[3] > 0x80 {
                opaque += 1;
            }
            if t[3] == 0 {
                clear += 1;
            }
            shades.insert((t[0], t[1], t[2], t[3]));
        }
        format!(
            "{texels} texels, {opaque} opaque, {clear} clear, {} distinct",
            shades.len()
        )
    }

    /// An icon is cut-out art: some of it is solid, some of it is clear, and the solid part is
    /// not one flat colour.
    fn has_content(rgba: &[u8]) -> bool {
        let texels = rgba.as_chunks::<4>().0;
        let mut total = 0usize;
        let mut opaque = 0usize;
        let mut clear = 0usize;
        let mut shades = std::collections::HashSet::new();
        for texel in texels {
            total += 1;
            if texel[3] > 0x80 {
                opaque += 1;
                shades.insert((texel[0], texel[1], texel[2]));
            }
            if texel[3] == 0 {
                clear += 1;
            }
        }
        opaque > total / 16 && clear > 0 && shades.len() > 8
    }
}
