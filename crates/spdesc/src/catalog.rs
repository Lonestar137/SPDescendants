//! The selectable characters and their skins, enumerated from the container index.
//!
//! A skin is a body mesh, its matching head mesh, and the `Material/<id>/` folder its textures live
//! in, all found under a character's `U/Skel/SKIN/<set>/` tree. A character also has independent
//! **head** variants (hairstyles/helmets, `…/Skel/SKIN/<set>/…_HEAD_…`) and **face** (makeup)
//! presets (the numbered folder holding the face albedo, at `…/A/Skel/SKIN/Makeup/<n>/` or
//! `…/A/Skel/SKIN/Makeup/Material/<n>/`). The skin data assets would name these directly, but
//! the local property mapping does not describe the struct the game ships, so this reads the
//! convention the asset names already encode instead.
//!
//! Body skins are limited to the `U/Skel` tree: the `A/Skel` bodies name their material slots
//! differently from their textures (`PartA`/`PartB` against `Body`/`Etc`/`Helmet`), so they cannot
//! be mapped without the material instances. Heads and makeup come from both trees.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value as Json;

/// Where every character's meshes live.
const MESH_ROOT: &str = "/M1/Content/Characters/PC/MESH/";

/// One selectable character and everything that can be swapped on it.
pub struct Character {
    pub id: String,
    pub name: String,
    /// Body skins (each with its default head).
    pub skins: Vec<Skin>,
    /// Independent head/hairstyle variants.
    pub heads: Vec<Variant>,
    /// Face (makeup) presets.
    pub faces: Vec<Variant>,
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

/// A swappable variant. `mesh` is the head mesh to use, or `None` for a face (makeup) preset, which
/// only changes the texture folder.
#[derive(Clone, PartialEq, Eq)]
pub struct Variant {
    pub label: String,
    pub mesh: Option<String>,
    pub material_dir: String,
}

/// Texture folders: char → `"{skel}/{set}"` → number → folder.
type Materials = HashMap<String, HashMap<String, HashMap<String, String>>>;

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

/// The folder name part of a `"{skel}/{set}"` key.
fn set_name(key: &str) -> &str {
    key.split_once('/').map(|(_, set)| set).unwrap_or(key)
}

/// The material folder for `key`/`number`, falling back to the same set under the other skeleton
/// tree (`U/CMN` ↔ `A/CMN`).
fn lookup<'a>(
    material: &'a HashMap<String, HashMap<String, String>>,
    key: &str,
    number: &str,
) -> Option<&'a String> {
    if let Some(dir) = material.get(key).and_then(|m| m.get(number)) {
        return Some(dir);
    }
    // A set can own a folder directly (`U/CMN` → `U/CMN/Material/001`) or through a parent
    // (`U/VAR` → `U/VAR/BODY/Material/000`).
    let prefix = format!("{key}/");
    if let Some(dir) = material
        .iter()
        .find(|(k, _)| k.starts_with(&prefix))
        .and_then(|(_, m)| m.get(number))
    {
        return Some(dir);
    }
    let (skel, set) = key.split_once('/')?;
    let other = format!("{}/{set}", if skel == "U" { "A" } else { "U" });
    if let Some(dir) = material.get(&other).and_then(|m| m.get(number)) {
        return Some(dir);
    }
    let other_prefix = format!("{other}/");
    material
        .iter()
        .find(|(k, _)| k.starts_with(&other_prefix))
        .and_then(|(_, m)| m.get(number))
}

/// Enumerate every character that has skins, with `names` supplying display names by id.
pub fn build(index: &HashMap<String, u32>, names: &HashMap<String, String>) -> Vec<Character> {
    let mut bodies: Materials = HashMap::new();
    let mut heads: Materials = HashMap::new();
    let mut material: Materials = HashMap::new();
    // char -> head/variant meshes as (key, number, mesh)
    let mut head_meshes: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    // char -> makeup folders
    let mut faces: HashMap<String, BTreeSet<String>> = HashMap::new();

    for path in index.keys() {
        let Some(rest) = path.strip_prefix(MESH_ROOT) else {
            continue;
        };
        if !path.ends_with(".uasset") {
            continue;
        }
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() < 6 || (parts[1] != "U" && parts[1] != "A") {
            continue;
        }
        let character = parts[0];
        let skel = parts[1];
        let dir = parts[4..parts.len() - 1].join("/");
        let key = format!("{skel}/{dir}");
        let stem = parts[parts.len() - 1].trim_end_matches(".uasset");
        // `BOS` is the boss set: its heads are rigged for bosses and float on a player body.
        if dir == "BOS" || dir.starts_with("BOS/") {
            continue;
        }

        // Makeup comes first: its presets sit under a folder literally named `Material`, which the
        // skin-material rule below would otherwise claim.
        if dir.contains("Makeup") {
            // A makeup preset is the numbered folder holding the face albedo. Its depth varies —
            // `…/Makeup/<preset>/…_C` for some characters, `…/Makeup/Material/<preset>/…_C` for
            // others — and the bare `Makeup`/`Material` folders hold only material instances, so
            // the albedo's own folder is the only reliable preset.
            let folder = dir.rsplit('/').next().unwrap_or_default();
            let lower = stem.to_ascii_lowercase();
            if !folder.is_empty()
                && folder.chars().all(|c| c.is_ascii_digit())
                && lower.contains("makeup")
                && lower.ends_with("_c")
            {
                faces
                    .entry(character.to_string())
                    .or_default()
                    .insert(folder_of(path));
            }
            continue;
        }
        if let Some((base, number)) = dir.split_once("/Material/") {
            if number.contains('/') {
                continue;
            }
            material
                .entry(character.to_string())
                .or_default()
                .entry(format!("{skel}/{base}"))
                .or_default()
                .entry(number.to_string())
                .or_insert_with(|| folder_of(path));
            continue;
        }
        if is_auxiliary(stem) {
            continue;
        }
        let Some(number) = skin_number(stem) else {
            continue;
        };
        if stem.contains("_BODY_") {
            // The `A/Skel` bodies do not map to their textures; only `U/Skel` body skins ship.
            if skel == "U" {
                insert(&mut bodies, character, &key, number, path);
            }
        } else if stem.contains("_HEAD_") {
            insert(&mut heads, character, &key, number, path);
            head_meshes.entry(character.to_string()).or_default().push((
                key.clone(),
                number.to_string(),
                path.clone(),
            ));
        }
    }

    let mut characters: Vec<Character> = Vec::new();
    for (id, sets) in &bodies {
        let mut skins: Vec<Skin> = Vec::new();
        for (key, by_number) in sets {
            for (number, body) in by_number {
                let Some(dir) = material.get(id).and_then(|m| lookup(m, key, number)) else {
                    continue;
                };
                let Some(head) = heads.get(id).and_then(|h| lookup(h, key, number)).cloned() else {
                    continue;
                };
                skins.push(Skin {
                    label: format!(
                        "{} {number}",
                        set_name(key).rsplit('/').next().unwrap_or(key)
                    ),
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
        skins.dedup_by(|a, b| a.label == b.label);

        let mut head_list: Vec<Variant> = Vec::new();
        if let Some(entries) = head_meshes.get(id) {
            let mut seen = BTreeSet::new();
            for (key, number, mesh) in entries {
                let label = format!(
                    "{} {number}",
                    set_name(key).rsplit('/').next().unwrap_or(key)
                );
                if !seen.insert(label.clone()) {
                    continue;
                }
                let dir = material
                    .get(id)
                    .and_then(|m| lookup(m, key, number))
                    .cloned()
                    .or_else(|| skins.first().map(|s| s.material_dir.clone()))
                    .unwrap_or_default();
                head_list.push(Variant {
                    label,
                    mesh: Some(mesh.clone()),
                    material_dir: dir,
                });
            }
            head_list.sort_by(|a, b| a.label.cmp(&b.label));
        }

        // A preset can ship under both layouts (`…/Makeup/006` and `…/Makeup/Material/006`); the
        // label is the preset number either way, so keep one of each.
        let mut face_list: Vec<Variant> = Vec::new();
        let mut face_labels = BTreeSet::new();
        for dir in faces.get(id).into_iter().flatten() {
            let label = dir.rsplit('/').next().unwrap_or("face").to_string();
            if face_labels.insert(label.clone()) {
                face_list.push(Variant {
                    label,
                    mesh: None,
                    material_dir: dir.clone(),
                });
            }
        }

        characters.push(Character {
            id: id.clone(),
            name: names.get(id).cloned().unwrap_or_else(|| format!("PC_{id}")),
            skins,
            heads: head_list,
            faces: face_list,
        });
    }
    characters.sort_by(|a, b| a.id.cmp(&b.id));
    characters
}

/// Meshes that are not renderable skins (`…_Pose`, cloth physics, collision proxies).
fn is_auxiliary(stem: &str) -> bool {
    stem.contains("ClothPhysics")
        || stem.contains("_PoseAsset")
        || stem.ends_with("_Pose")
        || stem.contains("_Clothing")
        || stem.contains("_Physics")
}

/// A texture's containing folder (its path without the file name).
fn folder_of(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default()
}

/// Insert a mesh under char → key → number.
fn insert(map: &mut Materials, character: &str, key: &str, number: &str, path: &str) {
    map.entry(character.to_string())
        .or_default()
        .entry(key.to_string())
        .or_default()
        .entry(number.to_string())
        .or_insert_with(|| path.to_string());
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
