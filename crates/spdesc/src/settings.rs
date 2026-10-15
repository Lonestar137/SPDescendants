//! What the player keeps between runs, under the gitignored config root beside the key, the
//! mapping and the roster: the character selection, the sound levels, and the career.
//!
//! Each is its own document, and each stands alone — a missing or malformed file means the default,
//! never a half-read state. The selection's entries are *names*, not list indices: the catalog is
//! rebuilt from the container every run (and its lists change as the data or the enumeration
//! changes), so an index is not stable between runs. A name that no longer resolves is simply
//! ignored, and the default stands.

use std::path::PathBuf;

use anyhow::Result;
use bevy::audio::{GlobalVolume, Volume};
use bevy::prelude::*;
use serde_json::{Map, Value};

/// `<config-root>/<name>`, the one place a document's path is decided.
fn document(name: &str) -> Option<PathBuf> {
    spdesc_data::key::config_root().map(|root| root.join(name))
}

/// A JSON object's number field, when it is one and is inside `range`.
fn number(
    map: &Map<String, Value>,
    key: &str,
    range: std::ops::RangeInclusive<f32>,
) -> Option<f32> {
    let value = map.get(key)?.as_f64()? as f32;
    range.contains(&value).then_some(value)
}

/// The character, skin, head and face the player last applied.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Selection {
    /// The character's asset id (`"015"`), which the container fixes.
    pub character: String,
    /// The skin, head and face labels as the menu shows them (`"CMN 001"`, `"Event 001"`, `"001"`).
    pub skin: String,
    pub head: String,
    pub face: String,
}

impl Selection {
    /// The four names, for a log line.
    pub fn describe(&self) -> String {
        format!(
            "character {} / skin {} / head {} / face {}",
            self.character, self.skin, self.head, self.face
        )
    }

    /// The saved selection, when the config root holds a well-formed one.
    pub fn load() -> Option<Selection> {
        Selection::from_json(&std::fs::read_to_string(path()?).ok()?)
    }

    /// Parse a selection document. Every field is required: a half-written file is no selection.
    pub fn from_json(text: &str) -> Option<Selection> {
        let Value::Object(map) = serde_json::from_str::<Value>(text).ok()? else {
            return None;
        };
        let field = |key: &str| map.get(key).and_then(Value::as_str).map(str::to_string);
        Some(Selection {
            character: field("character")?,
            skin: field("skin")?,
            head: field("head")?,
            face: field("face")?,
        })
    }

    /// The selection as a JSON document.
    pub fn to_json(&self) -> String {
        let mut map = Map::new();
        for (key, value) in [
            ("character", &self.character),
            ("skin", &self.skin),
            ("head", &self.head),
            ("face", &self.face),
        ] {
            map.insert(key.to_string(), Value::String(value.clone()));
        }
        let mut text =
            serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        text
    }

    /// Write the selection, creating the config root when it is missing. A missing config root
    /// (no `SPDESC_HOME`, not a checkout) is not an error: there is simply nowhere to keep it.
    pub fn save(&self) -> Result<()> {
        let Some(path) = path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json())?;
        Ok(())
    }
}

/// `<config-root>/selection.json`.
fn path() -> Option<PathBuf> {
    document("selection.json")
}

/// Keep Bevy's own master volume in step with the setting, which every cue obeys.
pub fn apply_volumes(settings: Res<Settings>, mut volume: ResMut<GlobalVolume>) {
    let wanted = Volume::Linear(settings.master.clamp(0.0, 1.0));
    if volume.volume != wanted {
        volume.volume = wanted;
    }
}

/// The sound levels, under the config root beside the selection.
///
/// Both are fractions of full volume. The master is applied through Bevy's own `GlobalVolume`, so
/// every cue obeys it; the gun-fire level trims the weapon's own fire on top of that.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Settings {
    pub master: f32,
    pub gun_fire: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            master: 1.0,
            gun_fire: 1.0,
        }
    }
}

impl Settings {
    /// The saved levels, or the defaults when the file is absent, malformed or out of range.
    pub fn load() -> Settings {
        std::fs::read_to_string(document("settings.json").unwrap_or_default())
            .ok()
            .and_then(|text| Settings::from_json(&text))
            .unwrap_or_default()
    }

    /// Parse a settings document. Both levels are required and must be inside `0..=1`: a level
    /// outside that range is a broken file, not a loud one.
    pub fn from_json(text: &str) -> Option<Settings> {
        let Value::Object(map) = serde_json::from_str::<Value>(text).ok()? else {
            return None;
        };
        Some(Settings {
            master: number(&map, "master", 0.0..=1.0)?,
            gun_fire: number(&map, "gun_fire", 0.0..=1.0)?,
        })
    }

    pub fn to_json(&self) -> String {
        let mut map = Map::new();
        for (key, value) in [("master", self.master), ("gun_fire", self.gun_fire)] {
            map.insert(key.to_string(), serde_json::json!(value));
        }
        let mut text =
            serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        text
    }

    /// Write the settings, creating the config root when it is missing.
    pub fn save(&self) -> Result<()> {
        let Some(path) = document("settings.json") else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json())?;
        Ok(())
    }

    /// A cue's volume: the master level times the cue's own base.
    pub fn cue(&self, base: f32) -> f32 {
        (self.master * base).clamp(0.0, 1.0)
    }

    /// The weapon's own fire: the master and the gun-fire level, times the cue's base.
    pub fn shot(&self, base: f32) -> f32 {
        (self.master * self.gun_fire * base).clamp(0.0, 1.0)
    }
}

/// The career as it is kept between runs: the rank, the prestige, and the experience inside the
/// rank. It is the player's, not a character's, which is how the game's own progression works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CareerSave {
    pub rank_index: u32,
    pub prestige: u32,
    pub xp_in_rank: u32,
}

impl CareerSave {
    /// The saved career, when the config root holds a well-formed one.
    pub fn load() -> Option<CareerSave> {
        CareerSave::from_json(&std::fs::read_to_string(document("career.json")?).ok()?)
    }

    /// Parse a career document. All three fields are required.
    pub fn from_json(text: &str) -> Option<CareerSave> {
        let Value::Object(map) = serde_json::from_str::<Value>(text).ok()? else {
            return None;
        };
        let field = |key: &str| -> Option<u32> { u32::try_from(map.get(key)?.as_u64()?).ok() };
        Some(CareerSave {
            rank_index: field("rank_index")?,
            prestige: field("prestige")?,
            xp_in_rank: field("xp_in_rank")?,
        })
    }

    /// A career is small and `Copy`, so this takes it by value, which is also the shape clippy
    /// wants from a `to_` method.
    pub fn to_json(self) -> String {
        let mut map = Map::new();
        for (key, value) in [
            ("rank_index", self.rank_index),
            ("prestige", self.prestige),
            ("xp_in_rank", self.xp_in_rank),
        ] {
            map.insert(key.to_string(), serde_json::json!(value));
        }
        let mut text =
            serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| "{}".to_string());
        text.push('\n');
        text
    }

    /// Write the career, creating the config root when it is missing.
    pub fn save(&self) -> Result<()> {
        let Some(path) = document("career.json") else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A selection survives a round trip through the file format, and a malformed or partial
    /// document is rejected rather than read as a half-made selection.
    #[test]
    fn a_selection_round_trips_and_partial_files_are_rejected() {
        let selection = Selection {
            character: "015".into(),
            skin: "CMN 001".into(),
            head: "Event 001".into(),
            face: "003".into(),
        };
        assert_eq!(
            Selection::from_json(&selection.to_json()).as_ref(),
            Some(&selection)
        );
        assert_eq!(Selection::from_json("{}"), None);
        assert_eq!(Selection::from_json("not json"), None);
        assert_eq!(
            Selection::from_json(r#"{"character":"015","skin":"CMN 001"}"#),
            None,
            "a file missing the head and face is not a selection"
        );
    }
}

#[cfg(test)]
mod document_tests {
    use super::*;

    /// The sound levels round trip, and a level outside `0..=1` is refused rather than obeyed.
    #[test]
    fn sound_levels_round_trip_and_are_range_checked() {
        let settings = Settings {
            master: 0.35,
            gun_fire: 0.8,
        };
        assert_eq!(
            Settings::from_json(&settings.to_json()).as_ref(),
            Some(&settings)
        );
        assert_eq!(Settings::from_json("{}"), None);
        assert_eq!(Settings::from_json(r#"{"master":1.0}"#), None);
        assert_eq!(
            Settings::from_json(r#"{"master":1.5,"gun_fire":1.0}"#),
            None,
            "a level above full volume is a broken file"
        );
        assert_eq!(
            Settings::from_json(r#"{"master":"loud","gun_fire":1.0}"#),
            None
        );
    }

    /// The shot obeys both levels; every other cue obeys the master alone.
    #[test]
    fn the_gun_fire_level_trims_only_the_shot() {
        let settings = Settings {
            master: 0.5,
            gun_fire: 0.5,
        };
        assert_eq!(settings.cue(0.6), 0.3);
        assert_eq!(settings.shot(0.6), 0.15);
        let silence = Settings {
            master: 0.0,
            gun_fire: 1.0,
        };
        assert_eq!(silence.shot(0.6), 0.0);
        assert_eq!(silence.cue(0.6), 0.0);
    }

    /// A career round trips, and a partial document is not a career.
    #[test]
    fn a_career_round_trips_and_partial_files_are_rejected() {
        let career = CareerSave {
            rank_index: 41,
            prestige: 3,
            xp_in_rank: 12_345,
        };
        assert_eq!(
            CareerSave::from_json(&career.to_json()).as_ref(),
            Some(&career)
        );
        assert_eq!(CareerSave::from_json("{}"), None);
        assert_eq!(
            CareerSave::from_json(r#"{"rank_index":1,"prestige":0}"#),
            None,
            "a career without its experience is not a career"
        );
    }
}
