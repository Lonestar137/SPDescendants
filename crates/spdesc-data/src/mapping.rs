//! Loading a property schema from a `.usmap`-style mapping in the local-state folder.
//!
//! Cooked unversioned property serialization carries no type information, so reading a value needs
//! a mapping of struct name → ordered properties → property types. That mapping is game-derived, so
//! it is *not* committed: it lives beside the key under `<config-root>/mappings/`. The shape read
//! here is the one FModel/UAssetAPI dumps (a JSON object of structs with `super` and `props`).

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde_json::Value as J;

use crate::property::{PropType, Property, Schema, StructDef};

/// The mapping file this crate reads under `<config-root>/mappings/`.
pub const MAPPING_FILE: &str = "usmap-fmodel-structs.json";

/// Where the mapping lives, when the local-state folder resolves.
pub fn mapping_path() -> Option<PathBuf> {
    crate::key::config_root().map(|root| root.join("mappings").join(MAPPING_FILE))
}

/// Load and parse the mapping. Errors when the folder or file is missing.
pub fn load() -> Result<Schema> {
    let path = mapping_path()
        .context("no local-state folder (set $SPDESC_HOME or build with the `dev` feature)")?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading mapping {}", path.display()))?;
    schema_from_mapping(&text)
}

/// Build a [`Schema`] from an FModel-style mapping document.
///
/// The mapping covers the whole game, so it names property types this reader has no wire form
/// for (delegates, `FieldPath`, `Text`, interfaces). A struct with any such property is skipped
/// rather than failing the load: the tables and asset classes this crate reads do not depend on
/// them, and an unreadable struct would only ever surface as a parse error where it is used.
pub fn schema_from_mapping(text: &str) -> Result<Schema> {
    let root: J = serde_json::from_str(text).context("mapping is not valid JSON")?;
    let object = root.as_object().context("mapping root is not an object")?;
    let mut schema = Schema::new();
    for (name, body) in object {
        let parent = body
            .get("super")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let props = body
            .get("props")
            .and_then(|v| v.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut properties = Vec::with_capacity(props.len());
        let mut readable = true;
        for p in props {
            let Some(pname) = p.get("name").and_then(|v| v.as_str()) else {
                readable = false;
                break;
            };
            match prop_type(p) {
                Ok(ty) => properties.push(Property {
                    name: pname.to_string(),
                    ty,
                }),
                Err(_) => {
                    readable = false;
                    break;
                }
            }
        }
        if !readable {
            continue;
        }
        schema.insert(StructDef {
            name: name.clone(),
            parent,
            properties,
        });
    }
    Ok(schema)
}

fn inner<'a>(p: &'a J, what: &str) -> Result<&'a J> {
    p.get("inner")
        .filter(|v| !v.is_null())
        .with_context(|| format!("{what} has no inner type"))
}

fn prop_type(p: &J) -> Result<PropType> {
    let t = p
        .get("type")
        .and_then(|v| v.as_str())
        .context("property has no type")?;
    Ok(match t {
        "Bool" => PropType::Bool,
        "Byte" | "Int8" => PropType::Byte,
        "Int16" => PropType::Int16,
        "UInt16" => PropType::UInt16,
        "Int" => PropType::Int,
        "UInt32" => PropType::UInt32,
        "Float" => PropType::Float,
        "Double" => PropType::Double,
        "Int64" => PropType::Int64,
        "UInt64" => PropType::UInt64,
        "Name" => PropType::Name,
        "Str" | "Utf8Str" | "AnsiStr" => PropType::Str,
        "Object" | "WeakObject" | "LazyObject" | "AssetObject" => PropType::Object,
        "SoftObject" | "SoftClass" => PropType::SoftObject,
        "Class" => PropType::Class,
        "Enum" => PropType::Enum(Box::new(prop_type(inner(p, "enum")?)?)),
        "Struct" => PropType::Struct(
            p.get("struct")
                .and_then(|v| v.as_str())
                .context("struct property has no struct name")?
                .to_string(),
        ),
        "Array" => PropType::Array(Box::new(prop_type(inner(p, "array")?)?)),
        "Set" => PropType::Set(Box::new(prop_type(inner(p, "set")?)?)),
        "Map" => PropType::Map(
            Box::new(prop_type(inner(p, "map key")?)?),
            Box::new(prop_type(
                p.get("value")
                    .filter(|v| !v.is_null())
                    .context("map has no value type")?,
            )?),
        ),
        "Optional" => PropType::Optional(Box::new(prop_type(inner(p, "optional")?)?)),
        other => bail!("unsupported property type {other}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structs_parents_and_nested_types() {
        let text = r#"{
            "Base": {"super": null, "props": [{"name": "Id", "type": "Int"}]},
            "Derived": {"super": "Base", "props": [
                {"name": "Names", "type": "Array", "inner": {"type": "Name"}},
                {"name": "Map", "type": "Map",
                 "inner": {"type": "Int"},
                 "value": {"type": "Struct", "struct": "Color"}},
                {"name": "Kind", "type": "Enum", "inner": {"type": "Byte"}}
            ]}
        }"#;
        let s = schema_from_mapping(text).unwrap();
        let names: Vec<&str> = s.chain("Derived").iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Names", "Map", "Kind", "Id"]);
        assert_eq!(
            s.get("Derived").unwrap().properties[0].ty,
            PropType::Array(Box::new(PropType::Name))
        );
        assert_eq!(
            s.get("Derived").unwrap().properties[1].ty,
            PropType::Map(
                Box::new(PropType::Int),
                Box::new(PropType::Struct("Color".into()))
            )
        );
    }

    #[test]
    fn skips_structs_with_an_unknown_property_type() {
        // The real mapping names types this reader cannot read (delegates, `FieldPath`, ...); the
        // loader must skip those structs and keep the rest.
        let s = schema_from_mapping(
            r#"{ "Good": {"props": [{"name":"Y","type":"Int"}]},
                 "Bad":  {"props": [{"name":"Z","type":"Fancy"}]} }"#,
        )
        .unwrap();
        assert!(s.get("Good").is_some());
        assert!(s.get("Bad").is_none());
    }
}
