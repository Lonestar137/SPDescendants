//! The player's saved selection, kept under the gitignored config root beside the key, the mapping
//! and the roster, so a choice survives a restart.
//!
//! The entries are *names*, not list indices: the catalog is rebuilt from the container every run
//! (and its lists change as the data or the enumeration changes), so an index is not stable between
//! runs. A name that no longer resolves is simply ignored, and the default stands.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Map, Value};

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
    spdesc_data::key::config_root().map(|root| root.join("selection.json"))
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
