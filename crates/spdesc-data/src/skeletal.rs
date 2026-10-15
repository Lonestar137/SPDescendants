//! Native (custom-serialized) structs used by `USkeletalMesh` and its members.
//!
//! Most UE structs serialize as an unversioned header plus their properties, which the schema
//! reader in [`crate::property`] handles. A few use C++ `CppStructOps` serializers with no header
//! at all — fixed-width math types, the `TPerPlatform*`/`TPerQualityLevel*` wrappers, and the
//! skeletal-mesh sampling built-data. This module implements the wire forms the research parser
//! (`skel-poc.py`) validated on the Ultimate Bunny body and head meshes.

use anyhow::{Result, bail};

use crate::property::{
    Names, NativeFn, Natives, Reader, Value, header_slots, read_unversioned_header,
};

/// Register every native struct serializer this reader knows.
pub fn register_natives(natives: &mut Natives) {
    let mut add = |name: &str, f: NativeFn| natives.insert(name, f);

    add("Vector3f", vec3f);
    add("Vector", vec3d);
    add("Vector2D", vec2d);
    add("Vector2f", vec2f);
    add("Quat", quat);
    add("Guid", guid);
    add("LinearColor", linear_color);
    add("Color", color);
    add("IntPoint", int_point);
    add("BoxSphereBounds", box_sphere_bounds);
    add("Transform", transform);

    add("PerPlatformFloat", per_platform_float);
    add("PerPlatformInt", per_platform_int);
    add("PerPlatformBool", per_platform_bool);
    add("PerQualityLevelInt", per_quality_level_int);
    add("PerQualityLevelBool", per_quality_level_int);

    add("SkeletalMeshSamplingLODBuiltData", sampling_lod);
    add("SkeletalMeshSamplingRegionBuiltData", sampling_region);
    add("SkeletalMeshSamplingBuiltData", sampling_built);
}

fn fields(pairs: &[(&str, Value)]) -> Value {
    Value::Struct(
        pairs
            .iter()
            .map(|(n, v)| ((*n).to_string(), v.clone()))
            .collect(),
    )
}

fn f(name: &str, v: f32) -> (&str, Value) {
    (name, Value::Float(v))
}

fn counted_array<T, F: Fn(&mut Reader) -> Result<T>>(
    r: &mut Reader,
    what: &str,
    read_one: F,
) -> Result<Vec<T>> {
    let n = r.i32()?;
    if !(0..=5_000_000).contains(&n) {
        bail!("{what}: bad count {n}");
    }
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        out.push(read_one(r)?);
    }
    Ok(out)
}

fn vec3f(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        f("x", r.f32()?),
        f("y", r.f32()?),
        f("z", r.f32()?),
    ]))
}

fn vec2f(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[f("x", r.f32()?), f("y", r.f32()?)]))
}

fn vec3d(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        ("x", Value::Double(r.f64()?)),
        ("y", Value::Double(r.f64()?)),
        ("z", Value::Double(r.f64()?)),
    ]))
}

fn vec2d(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        ("x", Value::Double(r.f64()?)),
        ("y", Value::Double(r.f64()?)),
    ]))
}

fn quat(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        ("x", Value::Double(r.f64()?)),
        ("y", Value::Double(r.f64()?)),
        ("z", Value::Double(r.f64()?)),
        ("w", Value::Double(r.f64()?)),
    ]))
}

fn guid(r: &mut Reader, _: &Names) -> Result<Value> {
    let mut bytes = [0u8; 16];
    for b in &mut bytes {
        *b = r.u8()?;
    }
    Ok(fields(&[("hex", Value::Text(hex(&bytes)))]))
}

fn linear_color(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        f("r", r.f32()?),
        f("g", r.f32()?),
        f("b", r.f32()?),
        f("a", r.f32()?),
    ]))
}

fn color(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        ("r", Value::UInt(u64::from(r.u8()?))),
        ("g", Value::UInt(u64::from(r.u8()?))),
        ("b", Value::UInt(u64::from(r.u8()?))),
        ("a", Value::UInt(u64::from(r.u8()?))),
    ]))
}

fn int_point(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        ("x", Value::Int(i64::from(r.i32()?))),
        ("y", Value::Int(i64::from(r.i32()?))),
    ]))
}

fn box_sphere_bounds(r: &mut Reader, _: &Names) -> Result<Value> {
    // 7 floats: origin, extent, radius.
    Ok(fields(&[
        f("ox", r.f32()?),
        f("oy", r.f32()?),
        f("oz", r.f32()?),
        f("ex", r.f32()?),
        f("ey", r.f32()?),
        f("ez", r.f32()?),
        f("radius", r.f32()?),
    ]))
}

fn transform(r: &mut Reader, _: &Names) -> Result<Value> {
    // 10 floats: quaternion xyzw, translation, scale.
    Ok(fields(&[
        f("qx", r.f32()?),
        f("qy", r.f32()?),
        f("qz", r.f32()?),
        f("qw", r.f32()?),
        f("tx", r.f32()?),
        f("ty", r.f32()?),
        f("tz", r.f32()?),
        f("sx", r.f32()?),
        f("sy", r.f32()?),
        f("sz", r.f32()?),
    ]))
}

fn per_platform_float(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        ("bCooked", Value::Bool(r.bool32()?)),
        f("value", r.f32()?),
    ]))
}

fn per_platform_int(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        ("bCooked", Value::Bool(r.bool32()?)),
        ("value", Value::Int(i64::from(r.i32()?))),
    ]))
}

fn per_platform_bool(r: &mut Reader, _: &Names) -> Result<Value> {
    Ok(fields(&[
        ("bCooked", Value::Bool(r.bool32()?)),
        ("value", Value::Bool(r.bool32()?)),
    ]))
}

fn per_quality_level_int(r: &mut Reader, _: &Names) -> Result<Value> {
    let cooked = r.bool32()?;
    let default = r.i32()?;
    let entries = counted_array(r, "per-quality entries", |r| {
        Ok((u64::from(r.u32()?), i64::from(r.i32()?)))
    })?;
    let map = entries
        .into_iter()
        .map(|(k, v)| (Value::UInt(k), Value::Int(v)))
        .collect();
    Ok(fields(&[
        ("bCooked", Value::Bool(cooked)),
        ("default", Value::Int(i64::from(default))),
        ("map", Value::Map(map)),
    ]))
}

/// `FWeightedRandomSampler`: float probabilities, int aliases, total weight.
fn weighted_sampler(r: &mut Reader) -> Result<Value> {
    let prob = counted_array(r, "sampler weights", |r| r.f32())?;
    let alias = counted_array(r, "sampler aliases", |r| Ok(i64::from(r.i32()?)))?;
    let total = r.f32()?;
    Ok(fields(&[
        (
            "prob",
            Value::Array(prob.into_iter().map(Value::Float).collect()),
        ),
        (
            "alias",
            Value::Array(alias.into_iter().map(Value::Int).collect()),
        ),
        ("total", Value::Float(total)),
    ]))
}

fn sampling_lod(r: &mut Reader, _: &Names) -> Result<Value> {
    weighted_sampler(r)
}

fn sampling_region(r: &mut Reader, _: &Names) -> Result<Value> {
    let triangles = counted_array(r, "region triangles", |r| Ok(i64::from(r.i32()?)))?;
    let bones = counted_array(r, "region bones", |r| Ok(i64::from(r.i32()?)))?;
    let sampler = weighted_sampler(r)?;
    let vertices = counted_array(r, "region vertices", |r| Ok(i64::from(r.i32()?)))?;
    Ok(fields(&[
        (
            "triangles",
            Value::Array(triangles.into_iter().map(Value::Int).collect()),
        ),
        (
            "bones",
            Value::Array(bones.into_iter().map(Value::Int).collect()),
        ),
        ("sampler", sampler),
        (
            "vertices",
            Value::Array(vertices.into_iter().map(Value::Int).collect()),
        ),
    ]))
}

/// `FSkeletalMeshSamplingBuiltData`: its own unversioned header, then the present arrays, then the
/// undocumented trailing int32 array this build appends.
fn sampling_built(r: &mut Reader, names: &Names) -> Result<Value> {
    let header = read_unversioned_header(r)?;
    if header_slots(&header) > 2 {
        bail!(
            "sampling built-data header has {} slots",
            header_slots(&header)
        );
    }
    let mut whole = Vec::new();
    let mut regions = Vec::new();
    let mut idx = 0usize;
    let mut mi = 0usize;
    for (skip, value_num) in &header.fragments {
        idx += skip;
        for _ in 0..*value_num {
            let slot = idx;
            idx += 1;
            let zero = header.is_zero.get(mi).copied().unwrap_or(false);
            mi += 1;
            if zero {
                continue;
            }
            let n = r.i32()?;
            if !(0..4096).contains(&n) {
                bail!("sampling built-data count {n}");
            }
            match slot {
                0 => {
                    for _ in 0..n {
                        whole.push(sampling_lod(r, names)?);
                    }
                }
                1 => {
                    for _ in 0..n {
                        regions.push(sampling_region(r, names)?);
                    }
                }
                _ => bail!("sampling built-data slot {slot}"),
            }
        }
    }
    let trailing = counted_array(r, "sampling built-data tail", |r| Ok(i64::from(r.i32()?)))?;
    Ok(fields(&[
        ("WholeMeshBuiltData", Value::Array(whole)),
        ("RegionBuiltData", Value::Array(regions)),
        (
            "TrailingInts",
            Value::Array(trailing.into_iter().map(Value::Int).collect()),
        ),
    ]))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Locate a `USkeleton`'s ordered reference-skeleton bone list in a cooked export.
///
/// A cooked `USkeleton` does not carry its `FReferenceSkeleton` as a tagged property: the ordered
/// bones live in the native tail as a run of `FMeshBoneInfo` records (an 8-byte `FMappedName` and an
/// `int32` parent). The run is located structurally rather than by a baked offset: it begins at a
/// record whose parent is `-1` (the single root) and continues while each parent is `-1` or an
/// earlier index. Returns `(name, parent)` in bone order.
pub fn reference_skeleton_order(
    export: &[u8],
    names: crate::property::Names,
) -> Option<Vec<(String, i32)>> {
    let record = |at: usize| -> Option<(String, i32)> {
        if at + 12 > export.len() {
            return None;
        }
        let raw = u32::from_le_bytes(export[at..at + 4].try_into().unwrap());
        let extra = u32::from_le_bytes(export[at + 4..at + 8].try_into().unwrap());
        let parent = i32::from_le_bytes(export[at + 8..at + 12].try_into().unwrap());
        let idx = (raw & 0x3fff_ffff) as usize;
        let table = if raw >> 30 != 0 {
            names.global
        } else {
            names.local
        };
        let base = table.get(idx)?;
        if base.is_empty() {
            return None;
        }
        let name = if extra > 0 {
            format!("{base}_{}", extra - 1)
        } else {
            base.clone()
        };
        Some((name, parent))
    };
    // The run begins at the single root (parent `-1`) and continues while each parent is `-1` or an
    // earlier index; the longest such run is the skeleton.
    let mut best = Vec::new();
    for start in 0..export.len().saturating_sub(12) {
        if i32::from_le_bytes(export[start + 8..start + 12].try_into().unwrap()) != -1 {
            continue;
        }
        let mut run = Vec::new();
        while let Some((name, parent)) = record(start + run.len() * 12) {
            if parent == -1 && !run.is_empty() {
                break;
            }
            if parent != -1 && !(parent >= 0 && (parent as usize) < run.len()) {
                break;
            }
            run.push((name, parent));
        }
        if run.len() > best.len() {
            best = run;
        }
    }
    (!best.is_empty()).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::IoStore;
    use crate::property::{Ctx, read_struct};
    use crate::zen::{GlobalNames, ZenPackage};
    /// Read the Ultimate Bunny body's `USkeletalMesh` property block with the Rust reader and    /// assert it lands exactly on the native-tail boundary the research parser established
    /// (`2_771_863`, `MESH-FORMAT.md`). This ties the runtime reader to the validated oracle.
    #[test]
    fn body_property_block_ends_on_the_native_tail() {
        let Some(paks) = crate::paks_dir() else {
            spdesc_game_core::TFD.skip("no TFD_DATA folder found");
            return;
        };
        let Some(key) = crate::key::aes_key() else {
            spdesc_game_core::TFD.skip_looked_in(
                "no container key (spdesc-config/keys.json)",
                &crate::key::config_root().into_iter().collect::<Vec<_>>(),
            );
            return;
        };
        let Ok(schema) = crate::mapping::load() else {
            spdesc_game_core::TFD.skip_looked_in(
                "no property mapping (spdesc-config/mappings)",
                &crate::mapping::mapping_path()
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
            return;
        };

        let mut store = IoStore::open(&paks, key).expect("open IoStore");
        let chunk = store.read_chunk(47475).expect("read body mesh chunk");
        let global = GlobalNames::read(&paks).expect("read global.ucas");
        let package = ZenPackage::parse(&chunk, &global).expect("parse body mesh package");
        let index = package
            .exports
            .iter()
            .position(|e| e.name == "PC_004_U_CMN_BODY_001")
            .expect("body mesh export present");
        let data = package.export_data(&chunk, index).expect("mesh data slice");

        let mut natives = Natives::new();
        register_natives(&mut natives);
        let ctx = Ctx {
            schema: &schema,
            names: Names {
                local: &package.local_names,
                global: &global.names,
            },
            natives: &natives,
        };
        let mut r = Reader::new(data);
        let value = read_struct(&mut r, "SkeletalMesh", &ctx).expect("read property block");
        let Value::Struct(fields) = value else {
            panic!("expected a struct")
        };
        assert!(fields.iter().any(|(n, _)| n == "SamplingInfo"));
        assert_eq!(
            r.offset(),
            2_771_863,
            "property block must end where the native tail begins"
        );
    }

    /// The `USkeleton` asset exposes its ordered reference-skeleton bones to the structural finder,
    /// which is what maps an animation's track indices onto mesh bones.
    #[test]
    fn skeleton_asset_exposes_its_reference_bone_order() {
        const SKELETON: &str = "/M1/Content/Characters/PCPC_Skeleton.uasset";
        let Some(paks) = crate::paks_dir() else {
            spdesc_game_core::TFD.skip("no TFD_DATA folder found");
            return;
        };
        let Some(key) = crate::key::aes_key() else {
            spdesc_game_core::TFD.skip_looked_in(
                "no container key (spdesc-config/keys.json)",
                &crate::key::config_root().into_iter().collect::<Vec<_>>(),
            );
            return;
        };
        let mut store = IoStore::open(&paks, key).expect("open IoStore");
        let index = store.directory_index().expect("directory index");
        let chunk = *index.get(SKELETON).expect("skeleton is indexed");
        let bytes = store
            .read_chunk(chunk as usize)
            .expect("read skeleton chunk");
        let global = GlobalNames::read(&paks).expect("global.ucas");
        let package = ZenPackage::parse(&bytes, &global).expect("parse package");
        let i = package
            .exports
            .iter()
            .position(|e| e.class.contains("Skeleton"))
            .expect("a Skeleton export");
        let names = crate::property::Names {
            local: &package.local_names,
            global: &global.names,
        };
        let order = package
            .export_data_variants(&bytes, i)
            .into_iter()
            .find_map(|data| reference_skeleton_order(data, names))
            .expect("the reference skeleton is found");
        assert!(
            order.len() > 3000,
            "the character skeleton has thousands of bones, got {}",
            order.len()
        );
        assert!(
            order.iter().any(|(n, _)| n == "Bip001-Head"),
            "the reference skeleton must contain the head bone"
        );
        assert!(
            order.iter().filter(|(_, p)| *p == -1).count() == 1,
            "exactly one root bone"
        );
    }
}
