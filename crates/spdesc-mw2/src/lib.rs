//! Reading Modern Warfare 2's shipped sound assets.
//!
//! The sound effects are not in the loose files: the weapon, footstep and hitmarker cues live
//! inside the fastfiles under `<MW2_DATA>/zone/english`, so reaching them means walking a zone the
//! way the game does. That container work is vendored from IW4L (`vendor/iw4/`); this crate is the
//! project's own reader on top of it — inflate a zone ([`open_zone`]), walk it for its sound
//! aliases and audio ([`read_sounds`]), and look a name up ([`SoundBank`]).
//!
//! Nothing here writes: a fastfile is opened read-only and its payloads are copied out. The audio
//! stays in memory or in the player's local state — never in the repository (`docs/METHOD.md`).

mod open;
mod sounds;

use std::path::{Path, PathBuf};

pub use open::{Error, Memory, ZoneImage, open_zone, parse_zone_image};
pub use sounds::{LoadedSound, PCM, SoundAlias, SoundSet, read_sounds};

/// The zones a Modern Warfare 2 install keeps its fastfiles in, under the install root.
pub const ZONE_DIR: &str = "zone/english";

/// The zones read for the effects the game plays itself, in order.
///
/// `common` is the always-loaded zone: the weapon, foley and impact sounds the game needs on every
/// map. `code_post_gfx` follows it for the interface and post-effect cues.
pub const EFFECT_ZONES: [&str; 2] = ["common", "code_post_gfx"];

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
    zones: Vec<(String, SoundSet)>,
    failures: Vec<(String, Error)>,
}

impl SoundBank {
    /// Read `zones` from the install at `install`.
    pub fn read(install: &Path, zones: &[&str]) -> SoundBank {
        let mut bank = SoundBank::default();
        for zone in zones {
            let path = zone_path(install, zone);
            if !path.is_file() {
                continue;
            }
            match open_zone(&path).and_then(|image| read_sounds(&image)) {
                Ok(set) => bank.zones.push(((*zone).to_owned(), set)),
                Err(error) => bank.failures.push(((*zone).to_owned(), error)),
            }
        }
        bank
    }

    /// The zone walks, for a caller that wants to report what was read.
    pub fn zones(&self) -> &[(String, SoundSet)] {
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
