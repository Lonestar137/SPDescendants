//! The selectable characters and their skins, enumerated from the container index.
//!
//! A skin is a body mesh, its matching head mesh, and the `Material/<id>/` folder its textures live
//! in, all found under a character's `U/Skel/SKIN/<set>/` tree. The skin data assets
//! (`M1CustomizingCharacterSkinDataAsset`) would name these directly, but the local property
//! mapping does not describe the struct the game ships, so this reads the convention the asset
//! names already encode instead.

use std::collections::HashMap;

use serde_json::Value as Json;

/// Where every character's meshes live.
const MESH_ROOT: &str = "/M1/Content/Characters/PC/MESH/";

/// One selectable character and its skins.
pub struct Character {
    pub id: String,
    pub name: String,
    pub skins: Vec<Skin>,
}

/// A body + head skin and the folder its textures live in.
#[derive(Clone, PartialEq, Eq)]
pub struct Skin {
    /// Display label, e.g. `CMN 001`.
    pub label: String,
    /// Body mesh package path.
    pub body: String,
    /// Head mesh package path.
    pub head: String,
    /// Folder holding the skin's `Texture*.uasset` files.
    pub material_dir: String,
}

/// The trailing run of digits in a file stem (`001PC_…_BODY_001` and `BODYPC_…_VAR_BODY_000` both
/// resolve to their skin number).
fn skin_number(stem: &str) -> Option<&str> {
    let bytes = stem.as_bytes();
    let mut end = bytes.len();
    while end > 0 && !bytes[end - 1].is_ascii_digit() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && bytes[start - 1].is_ascii_digit() {
        start -= 1;
    }
    (start < end).then(|| &stem[start..end])
}

/// Enumerate every character that has skins, with `names` supplying display names by id.
pub fn build(index: &HashMap<String, u32>, names: &HashMap<String, String>) -> Vec<Character> {
    // char id -> set -> skin number -> body/head mesh
    let mut bodies: HashMap<String, HashMap<String, HashMap<String, String>>> = HashMap::new();
    let mut heads: HashMap<String, HashMap<String, HashMap<String, String>>> = HashMap::new();
    // char id -> material set -> skin number -> folder
    let mut material: HashMap<String, HashMap<String, HashMap<String, String>>> = HashMap::new();

    for key in index.keys() {
        let Some(rest) = key.strip_prefix(MESH_ROOT) else {
            continue;
        };
        if !key.ends_with(".uasset") {
            continue;
        }
        let parts: Vec<&str> = rest.split('/').collect();
        // {char}/U/Skel/SKIN/…
        if parts.len() < 6 || parts[1] != "U" || parts[2] != "Skel" || parts[3] != "SKIN" {
            continue;
        }
        let (character, dir, stem) = (
            parts[0],
            parts[4..parts.len() - 1].join("/"),
            parts[parts.len() - 1].trim_end_matches(".uasset"),
        );

        if let Some((set, id)) = dir.split_once("/Material/") {
            if id.contains('/') {
                continue;
            }
            material
                .entry(character.to_string())
                .or_default()
                .entry(set.to_string())
                .or_default()
                .entry(id.to_string())
                .or_insert_with(|| format!("{MESH_ROOT}{character}/U/Skel/SKIN/{dir}"));
            continue;
        }
        if stem.contains("ClothPhysics")
            || stem.contains("_PoseAsset")
            || stem.contains("_Clothing")
        {
            continue;
        }
        let Some(id) = skin_number(stem) else {
            continue;
        };
        let target = if stem.contains("_BODY_") {
            &mut bodies
        } else if stem.contains("_HEAD_") {
            &mut heads
        } else {
            continue;
        };
        target
            .entry(character.to_string())
            .or_default()
            .entry(dir.to_string())
            .or_default()
            .entry(id.to_string())
            .or_insert_with(|| key.clone());
    }

    let mut characters: Vec<Character> = Vec::new();
    for (id, sets) in &bodies {
        let mut skins: Vec<Skin> = Vec::new();
        for (set, by_id) in sets {
            for (number, body) in by_id {
                // The material folder is the one whose set matches this mesh's set and whose id is
                // the skin number (`CMN` → `CMN/Material/001`, `VAR` → `VAR/BODY/Material/000`).
                let Some(dir) = material.get(id).and_then(|sets| {
                    sets.iter()
                        .find(|(mset, by_id)| {
                            (mset.as_str() == set.as_str() || mset.starts_with(&format!("{set}/")))
                                && by_id.contains_key(number)
                        })
                        .and_then(|(_, by_id)| by_id.get(number))
                }) else {
                    continue;
                };
                let Some(head) = heads
                    .get(id)
                    .and_then(|s| s.get(set))
                    .and_then(|b| b.get(number))
                    .cloned()
                else {
                    continue;
                };
                skins.push(Skin {
                    label: format!("{set} {number}"),
                    body: body.clone(),
                    head,
                    material_dir: dir.clone(),
                });
            }
        }
        if skins.is_empty() {
            continue;
        }
        skins.sort_by(|a, b| a.label.cmp(&b.label));
        characters.push(Character {
            id: id.clone(),
            name: names.get(id).cloned().unwrap_or_else(|| format!("PC_{id}")),
            skins,
        });
    }
    characters.sort_by(|a, b| a.id.cmp(&b.id));
    characters
}

/// Load character display names from `<config-root>/roster.json` (`{"004": "Bunny"}`), if present.
pub fn load_names() -> HashMap<String, String> {
    let Some(path) = spdesc_data::key::config_root().map(|root| root.join("roster.json")) else {
        return HashMap::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    let Ok(Json::Object(map)) = serde_json::from_str::<Json>(&text) else {
        return HashMap::new();
    };
    map.into_iter()
        .filter_map(|(id, name)| name.as_str().map(|n| (id, n.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The skin number is the trailing digit run, so `VAR`'s `BODYPC_…_000` and `CMN`'s
    /// `001PC_…_001` both resolve.
    #[test]
    fn skin_numbers_come_from_the_trailing_digits() {
        assert_eq!(skin_number("001PC_004_U_CMN_BODY_001"), Some("001"));
        assert_eq!(skin_number("BODYPC_004_U_VAR_BODY_000"), Some("000"));
        assert_eq!(skin_number("no_digits_here"), None);
    }
}
