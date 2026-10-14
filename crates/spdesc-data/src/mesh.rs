//! `USkeletalMesh` render data: the native tail after the property block.
//!
//! The property block ends at a native-defined boundary; from there a `USkeletalMesh` writes, in
//! order, `FStripDataFlags`, `FBoxSphereBounds`, the materials, the reference skeleton, a cooked
//! flag, and one `FSkeletalMeshLODRenderData` per LOD (sections, indices, positions, tangents,
//! UVs, skin weights, colours), then a small tail. Layout and field rules follow the research
//! parser (`skel-poc.py`, `MESH-FORMAT.md`) that was validated to the exact export boundary on the
//! Ultimate Bunny body and head.

use anyhow::{Context, Result, bail};

use crate::property::{Ctx, Names, Reader, Value, read_struct};

/// One `FMeshBoneInfo`.
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    pub parent: i32,
}

/// One bind-pose `FTransform` (10 floats: quaternion xyzw, translation, scale).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub quat: [f32; 4],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

/// One `FSkeletalMaterial`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Material {
    pub slot_name: String,
    pub imported_slot_name: Option<String>,
}

/// One `FSkelMeshRenderSection`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub material_index: i16,
    pub base_index: i32,
    pub num_triangles: i32,
    pub base_vertex_index: u32,
    pub num_vertices: i32,
    pub max_bone_influences: i32,
    pub bone_map: Vec<u16>,
    pub disabled: bool,
}

/// One cooked LOD's render data.
#[derive(Debug, Clone)]
pub struct MeshLod {
    pub sections: Vec<Section>,
    /// Global vertex indices (each section's range spans its own vertex range).
    pub indices: Vec<u32>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Skeleton bone indices, four per vertex (mapped through the owning section's bone map).
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub has_vertex_colors: bool,
}

/// A parsed `USkeletalMesh` (geometry + skeleton + materials).
#[derive(Debug, Clone)]
pub struct SkeletalMesh {
    pub bounds_origin: [f32; 3],
    pub bounds_extent: [f32; 3],
    pub bounds_radius: f32,
    pub materials: Vec<Material>,
    pub bones: Vec<Bone>,
    pub poses: Vec<Transform>,
    pub lods: Vec<MeshLod>,
}

/// Read the whole export: property block, then the native tail. The reader must be positioned at
/// the start of the export's data.
pub fn parse(r: &mut Reader, ctx: &Ctx) -> Result<SkeletalMesh> {
    let props = read_struct(r, "SkeletalMesh", ctx).context("SkeletalMesh property block")?;
    let has_vertex_colors = match &props {
        Value::Struct(fields) => fields
            .iter()
            .any(|(n, v)| n == "bHasVertexColors" && matches!(v, Value::Bool(true))),
        _ => false,
    };

    let _strip_flags = (r.u8()?, r.u8()?);
    let (bounds_origin, bounds_extent, bounds_radius) = read_bounds(r)?;

    let material_count = r.i32()?;
    if !(0..100).contains(&material_count) {
        bail!("skeletal materials count {material_count}");
    }
    let mut materials = Vec::with_capacity(material_count as usize);
    for _ in 0..material_count {
        materials.push(read_material(r, ctx.names)?);
    }

    let (bones, poses) = read_reference_skeleton(r, ctx.names)?;

    let cooked = r.bool32()?;
    if !cooked {
        bail!("mesh has no cooked render data");
    }
    let lod_count = r.i32()?;
    if !(0..=16).contains(&lod_count) {
        bail!("lod count {lod_count}");
    }
    let mut lods = Vec::with_capacity(lod_count as usize);
    for lod in 0..lod_count {
        lods.push(read_lod(r, has_vertex_colors, &bones).with_context(|| format!("LOD {lod}"))?);
    }

    // Tail: NumInlinedLODs, NumNonOptionalLODs, then a dummy FPackageIndex array.
    let _num_inlined = r.u8()?;
    let _num_non_optional = r.u8()?;
    let dummy = r.i32()?;
    if !(0..100_000).contains(&dummy) {
        bail!("dummy object count {dummy}");
    }
    r.skip(dummy as usize * 4)?;

    Ok(SkeletalMesh {
        bounds_origin,
        bounds_extent,
        bounds_radius,
        materials,
        bones,
        poses,
        lods,
    })
}

fn read_bounds(r: &mut Reader) -> Result<([f32; 3], [f32; 3], f32)> {
    let origin = [r.f32()?, r.f32()?, r.f32()?];
    let extent = [r.f32()?, r.f32()?, r.f32()?];
    let radius = r.f32()?;
    Ok((origin, extent, radius))
}

fn read_material(r: &mut Reader, names: Names) -> Result<Material> {
    // UObject* Material, FName slot name, bool bSerializeImportedMaterialSlotName, and
    // FMeshUVChannelInfo (two bools + four floats) = 40 B.
    let _material = r.i32()?;
    let slot_name = names.read(r)?;
    let serialize_imported = r.bool32()?;
    let imported_slot_name = if serialize_imported {
        Some(names.read(r)?)
    } else {
        None
    };
    let _uv_initialized = r.bool32()?;
    let _uv_override = r.bool32()?;
    r.skip(16)?;
    Ok(Material {
        slot_name,
        imported_slot_name,
    })
}

/// Read an `FReferenceSkeleton`: the bones (name + parent) and their bind-pose locals, then the
/// name-to-index map.
fn read_reference_skeleton(r: &mut Reader, names: Names) -> Result<(Vec<Bone>, Vec<Transform>)> {
    let count = r.i32()?;
    if !(0..5000).contains(&count) {
        bail!("bone count {count}");
    }
    let mut bones = Vec::with_capacity(count as usize);
    for _ in 0..count {
        bones.push(Bone {
            name: names.read(r)?,
            parent: r.i32()?,
        });
    }
    let pose_count = r.i32()?;
    if pose_count != count {
        bail!("pose count {pose_count} != bone count {count}");
    }
    let mut poses = Vec::with_capacity(pose_count as usize);
    for _ in 0..pose_count {
        poses.push(Transform {
            quat: [r.f32()?, r.f32()?, r.f32()?, r.f32()?],
            translation: [r.f32()?, r.f32()?, r.f32()?],
            scale: [r.f32()?, r.f32()?, r.f32()?],
        });
    }
    let map_count = r.i32()?;
    if map_count != count {
        bail!("name map count {map_count} != bone count {count}");
    }
    for _ in 0..map_count {
        let _name = names.read(r)?;
        let _index = r.i32()?;
    }
    Ok((bones, poses))
}

fn read_section(r: &mut Reader) -> Result<Section> {
    let _flags = (r.u8()?, r.u8()?);
    let material_index = r.i16()?;
    let base_index = r.i32()?;
    let num_triangles = r.i32()?;
    let _recompute_tangent = r.bool32()?;
    let _mask_channel = r.u8()?;
    let _cast_shadow = r.bool32()?;
    let _visible_in_ray_tracing = r.bool32()?;
    let base_vertex_index = r.u32()?;
    // ClothMappingDataLODs: an array of arrays of FMeshToMeshVertData (64 B each).
    let outer = r.i32()?;
    if !(0..64).contains(&outer) {
        bail!("cloth outer {outer}");
    }
    for _ in 0..outer {
        let inner = r.i32()?;
        if !(0..20_000_000).contains(&inner) {
            bail!("cloth inner {inner}");
        }
        r.skip(inner as usize * 64)?;
    }
    let bone_map_count = r.i32()?;
    if !(0..20_000).contains(&bone_map_count) {
        bail!("bone map {bone_map_count}");
    }
    let mut bone_map = Vec::with_capacity(bone_map_count as usize);
    for _ in 0..bone_map_count {
        bone_map.push(r.u16()?);
    }
    let num_vertices = r.i32()?;
    let packed = r.i32()?;
    let max_bone_influences = packed & 0x7FFF_FFFF;
    let _cloth_asset_index = r.i16()?;
    r.skip(20)?; // FClothingSectionData (FGuid + int32)
    skip_fixed_array(r, 4)?; // FDuplicatedVerticesBuffer: DupVertData
    skip_fixed_array(r, 8)?; // FDuplicatedVerticesBuffer: DupVertIndexData
    let disabled = r.bool32()?;
    Ok(Section {
        material_index,
        base_index,
        num_triangles,
        base_vertex_index,
        num_vertices,
        max_bone_influences,
        bone_map,
        disabled,
    })
}

struct SkinWeights {
    variable: bool,
    max_influences: u32,
    num_vertices: u32,
    use16_index: bool,
    use16_weight: bool,
    data: Vec<u8>,
    lookup: Vec<u32>,
}

fn read_skin_weights(r: &mut Reader) -> Result<SkinWeights> {
    let _flags = (r.u8()?, r.u8()?);
    let variable = r.bool32()?;
    let max_influences = r.u32()?;
    let _num_bones = r.u32()?;
    let num_vertices = r.u32()?;
    let use16_index = r.bool32()?;
    let use16_weight = r.bool32()?;
    let (data_size, data) = bulk_data(r)?;
    if data_size != 1 {
        bail!("skin weight element size {data_size}");
    }
    let _lookup_flags = (r.u8()?, r.u8()?);
    let _num_lookup = r.i32()?;
    let (lookup_size, lookup_bytes) = bulk_data(r)?;
    if lookup_size != 4 {
        bail!("skin weight lookup element size {lookup_size}");
    }
    let lookup = lookup_bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    Ok(SkinWeights {
        variable,
        max_influences,
        num_vertices,
        use16_index,
        use16_weight,
        data,
        lookup,
    })
}

/// Decode `FSkinWeightInfo` for every vertex: bone indices first, then weights (not interleaved).
fn decode_skin_weights(sw: &SkinWeights) -> Result<Vec<Vec<(u16, u16)>>> {
    let idx_size = if sw.use16_index { 2 } else { 1 };
    let weight_size = if sw.use16_weight { 2 } else { 1 };
    let mut out = Vec::with_capacity(sw.num_vertices as usize);
    for i in 0..sw.num_vertices as usize {
        let (pos, count) = if sw.variable {
            let entry = *sw
                .lookup
                .get(i)
                .context("skin weight lookup shorter than vertex count")?;
            ((entry >> 8) as usize, (entry & 0xff) as usize)
        } else {
            (
                i * sw.max_influences as usize * (idx_size + weight_size),
                sw.max_influences as usize,
            )
        };
        let mut inf = Vec::with_capacity(count);
        let mut ipos = pos;
        let mut wpos = pos + count * idx_size;
        for _ in 0..count {
            let bone = if idx_size == 2 {
                u16::from_le_bytes([sw.data[ipos], sw.data[ipos + 1]])
            } else {
                u16::from(sw.data[ipos])
            };
            ipos += idx_size;
            let weight = if weight_size == 2 {
                u16::from_le_bytes([sw.data[wpos], sw.data[wpos + 1]])
            } else {
                u16::from(sw.data[wpos])
            };
            wpos += weight_size;
            inf.push((bone, weight));
        }
        out.push(inf);
    }
    Ok(out)
}

fn read_lod(r: &mut Reader, has_vertex_colors: bool, bones: &[Bone]) -> Result<MeshLod> {
    let _flags = (r.u8()?, r.u8()?);
    let cooked_out = r.bool32()?;
    let inlined = r.bool32()?;
    let required = r.i32()?;
    if !(0..5000).contains(&required) {
        bail!("required bones {required}");
    }
    r.skip(required as usize * 2)?;
    if cooked_out {
        bail!("LOD is cooked out");
    }
    if !inlined {
        bail!("LOD data is not inlined (bulk form not supported yet)");
    }

    let section_count = r.i32()?;
    if !(0..256).contains(&section_count) {
        bail!("section count {section_count}");
    }
    let mut sections = Vec::with_capacity(section_count as usize);
    for _ in 0..section_count {
        sections.push(read_section(r)?);
    }
    let active = r.i32()?;
    if !(0..5000).contains(&active) {
        bail!("active bone count {active}");
    }
    r.skip(active as usize * 2)?;
    let buffer_size = r.u32()?;

    let stream_start = r.offset();
    let _stream_flags = (r.u8()?, r.u8()?);

    let data_size = r.u8()?;
    if data_size != 2 && data_size != 4 {
        bail!("index element size {data_size}");
    }
    let (index_esz, index_bytes) = bulk_data(r)?;
    if index_esz as u8 != data_size {
        bail!("index element size {index_esz} != {data_size}");
    }
    let indices: Vec<u32> = if data_size == 2 {
        index_bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u32::from(u16::from_le_bytes(*c)))
            .collect()
    } else {
        index_bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect()
    };

    let stride = r.i32()?;
    let num_vertices = r.i32()?;
    let (pos_esz, pos_bytes) = bulk_data(r)?;
    if pos_esz != 12 {
        bail!("position stride {stride} / element size {pos_esz}");
    }
    let positions: Vec<[f32; 3]> = pos_bytes
        .as_chunks::<12>()
        .0
        .iter()
        .map(|c| {
            [
                f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                f32::from_le_bytes([c[8], c[9], c[10], c[11]]),
            ]
        })
        .collect();
    if positions.len() != num_vertices as usize {
        bail!("position count {} != {num_vertices}", positions.len());
    }

    // FStaticMeshVertexBuffer: strip flags, metadata, tangent array, UV array.
    let _tan_flags = (r.u8()?, r.u8()?);
    let num_tex_coords = r.i32()?;
    let num_verts2 = r.i32()?;
    let full_precision_uvs = r.bool32()?;
    let high_precision_tangents = r.bool32()?;
    let tangent_esz = r.i32()?;
    let tangent_count = r.i32()?;
    if tangent_count != num_verts2 {
        bail!("tangent count {tangent_count} != {num_verts2}");
    }
    let tangent_bytes = if high_precision_tangents { 16 } else { 8 };
    if tangent_esz != tangent_bytes {
        bail!("tangent element size {tangent_esz} != {tangent_bytes}");
    }
    let mut normals = Vec::with_capacity(tangent_count as usize);
    for _ in 0..tangent_count {
        let _tangent_x = r.u32()?;
        let tangent_z = r.u32()?;
        normals.push(unpack_normal(tangent_z ^ 0x8080_8080));
    }
    let uv_esz = r.i32()?;
    let uv_count = r.i32()?;
    let uv_bytes = if full_precision_uvs { 8 } else { 4 };
    if uv_esz != uv_bytes * num_tex_coords {
        bail!("uv element size {uv_esz}");
    }
    let expected_uv_verts = if uv_count == num_verts2 * num_tex_coords {
        num_verts2
    } else if num_verts2 > 0 {
        num_verts2 + num_tex_coords % 2
    } else {
        0
    };
    if uv_count != expected_uv_verts * num_tex_coords {
        bail!("uv count {uv_count} != {expected_uv_verts}*{num_tex_coords}");
    }
    let mut uvs = Vec::with_capacity(expected_uv_verts as usize);
    for _ in 0..expected_uv_verts {
        if full_precision_uvs {
            uvs.push([r.f32()?, r.f32()?]);
            r.skip((num_tex_coords as usize - 1) * 8)?;
        } else {
            uvs.push([half_to_f32(r.u16()?), half_to_f32(r.u16()?)]);
            r.skip((num_tex_coords as usize - 1) * 4)?;
        }
    }

    let skin = read_skin_weights(r)?;
    if skin.num_vertices != num_vertices as u32 {
        bail!(
            "skin weight vertex count {} != {num_vertices}",
            skin.num_vertices
        );
    }
    let decoded = decode_skin_weights(&skin)?;

    if has_vertex_colors {
        let _color_flags = (r.u8()?, r.u8()?);
        let _color_stride = r.i32()?;
        let color_count = r.i32()?;
        if color_count > 0 {
            let (_esz, _bytes) = bulk_data(r)?;
        }
    }

    // Everything up to `stream_start + buffer_size` belongs to this LOD.
    let stream_end = stream_start + buffer_size as usize;
    if r.offset() > stream_end {
        bail!(
            "LOD streamed data overran its buffer ({} > {stream_end})",
            r.offset()
        );
    }
    r.skip(stream_end - r.offset())?;

    // Map each vertex's weight bone indices through its owning section's bone map.
    let mut owner = vec![usize::MAX; positions.len()];
    for (si, s) in sections.iter().enumerate() {
        let start = s.base_vertex_index as usize;
        let end = start + s.num_vertices as usize;
        for slot in owner
            .get_mut(start..end)
            .with_context(|| format!("section {si} vertex range {start}..{end} out of bounds"))?
        {
            *slot = si;
        }
    }
    if let Some(bad) = owner.iter().position(|s| *s == usize::MAX) {
        bail!("vertex {bad} is not covered by any section");
    }
    let mut joints = Vec::with_capacity(positions.len());
    let mut weights = Vec::with_capacity(positions.len());
    for (v, inf) in decoded.iter().enumerate() {
        let bone_map = &sections[owner[v]].bone_map;
        let mut inf = inf.clone();
        inf.sort_by_key(|a| std::cmp::Reverse(a.1));
        inf.truncate(4);
        let total: u32 = inf.iter().map(|(_, w)| u32::from(*w)).sum();
        let total = total.max(1);
        let mut js = [0u16; 4];
        let mut ws = [0f32; 4];
        for (k, (bone, weight)) in inf.iter().enumerate() {
            let mapped = *bone_map
                .get(*bone as usize)
                .with_context(|| format!("vertex {v}: weight bone {bone} outside its bone map"))?;
            if mapped as usize >= bones.len() {
                bail!(
                    "vertex {v}: bone map index {mapped} >= bone count {}",
                    bones.len()
                );
            }
            js[k] = mapped;
            ws[k] = f32::from(*weight) / total as f32;
        }
        joints.push(js);
        weights.push(ws);
    }

    Ok(MeshLod {
        sections,
        indices,
        positions,
        normals,
        uvs,
        joints,
        weights,
        has_vertex_colors,
    })
}

/// A bulk array: an element size and count, then the elements. Returns `(element_size, bytes)`.
fn bulk_data(r: &mut Reader) -> Result<(i32, Vec<u8>)> {
    let element_size = r.i32()?;
    let count = r.i32()?;
    if !(0..50_000_000).contains(&count) {
        bail!("bulk count {count}");
    }
    let len = count as usize * element_size.max(0) as usize;
    let start = r.offset();
    r.skip(len)?;
    // Read the bytes back out of the same reader's backing slice.
    Ok((element_size, r.slice(start, len).to_vec()))
}

/// `SkipFixedArray(n)`: an int32 count then `count * n` bytes.
fn skip_fixed_array(r: &mut Reader, element_size: usize) -> Result<()> {
    let count = r.i32()?;
    if !(0..50_000_000).contains(&count) {
        bail!("fixed array count {count}");
    }
    r.skip(count as usize * element_size)?;
    Ok(())
}

/// Decompress one `FPackedNormal` component triple (byte per axis) into a unit-ish vector.
fn unpack_normal(v: u32) -> [f32; 3] {
    [
        (v & 0xff) as f32 / 127.5 - 1.0,
        ((v >> 8) & 0xff) as f32 / 127.5 - 1.0,
        ((v >> 16) & 0xff) as f32 / 127.5 - 1.0,
    ]
}

/// IEEE-754 half to single.
fn half_to_f32(bits: u16) -> f32 {
    let sign = (bits >> 15) & 1;
    let exponent = (bits >> 10) & 0x1f;
    let mantissa = bits & 0x3ff;
    let magnitude = if exponent == 0 {
        f32::from(mantissa) * 2f32.powi(-24)
    } else if exponent == 31 {
        if mantissa == 0 {
            f32::INFINITY
        } else {
            f32::NAN
        }
    } else {
        (1.0 + f32::from(mantissa) / 1024.0) * 2f32.powi(i32::from(exponent) - 15)
    };
    if sign == 1 { -magnitude } else { magnitude }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::IoStore;
    use crate::property::{Natives, Reader};
    use crate::zen::{GlobalNames, ZenPackage};

    fn parse_or_skip(chunk_id: usize, export_name: &str) -> Option<(SkeletalMesh, usize)> {
        let Some(paks) = crate::paks_dir() else {
            spdesc_game_core::skipped("no game data found", &spdesc_game_core::candidates());
            return None;
        };
        let Some(key) = crate::key::aes_key() else {
            spdesc_game_core::skipped(
                "no container key (spdesc-config/keys.json)",
                &crate::key::config_root().into_iter().collect::<Vec<_>>(),
            );
            return None;
        };
        let Ok(schema) = crate::mapping::load() else {
            spdesc_game_core::skipped(
                "no property mapping (spdesc-config/mappings)",
                &crate::mapping::mapping_path()
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
            return None;
        };
        let mut store = IoStore::open(&paks, key).expect("open IoStore");
        let chunk = store.read_chunk(chunk_id).expect("read chunk");
        let global = GlobalNames::read(&paks).expect("read global.ucas");
        let package = ZenPackage::parse(&chunk, &global).expect("parse package");
        let index = package
            .exports
            .iter()
            .position(|e| e.name == export_name)
            .expect("export present");
        let variants = package.export_data_variants(&chunk, index);
        assert!(!variants.is_empty(), "export data range is valid");
        let len = variants[0].len();
        let mut natives = Natives::new();
        crate::skeletal::register_natives(&mut natives);
        let ctx = Ctx {
            schema: &schema,
            names: Names {
                local: &package.local_names,
                global: &global.names,
            },
            natives: &natives,
        };
        // The head stores its exports in reverse physical order, so try each candidate range.
        let mut last = None;
        for data in variants {
            let mut r = Reader::new(data);
            match parse(&mut r, &ctx) {
                Ok(mesh) => {
                    assert_eq!(r.offset(), len, "the parse must consume the exact export");
                    return Some((mesh, len));
                }
                Err(error) => last = Some(error),
            }
        }
        panic!("no candidate range parsed: {last:?}");
    }

    fn assert_bounds_match(lod: &MeshLod, mesh: &SkeletalMesh) {
        for axis in 0..3 {
            let lo = lod
                .positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::MAX, f32::min);
            let hi = lod
                .positions
                .iter()
                .map(|p| p[axis])
                .fold(f32::MIN, f32::max);
            let center = (lo + hi) / 2.0;
            let half = (hi - lo) / 2.0;
            assert!(
                (center - mesh.bounds_origin[axis]).abs() < 1.0,
                "axis {axis}: position centre {center} vs ImportedBounds {}",
                mesh.bounds_origin[axis]
            );
            assert!(
                (half - mesh.bounds_extent[axis]).abs() < mesh.bounds_extent[axis] * 0.05 + 1.0,
                "axis {axis}: half-extent {half} vs ImportedBounds {}",
                mesh.bounds_extent[axis]
            );
        }
    }

    #[test]
    fn body_mesh_parses_to_the_exact_boundary() {
        let Some((mesh, len)) = parse_or_skip(47475, "PC_004_U_CMN_BODY_001") else {
            return;
        };
        assert_eq!(len, 12_376_061);
        assert_eq!(mesh.bones.len(), 289);
        assert_eq!(mesh.materials.len(), 5);
        assert_eq!(mesh.lods.len(), 4);

        let lod0 = &mesh.lods[0];
        assert_eq!(lod0.sections.len(), 5);
        assert_eq!(lod0.positions.len(), 82_709);
        assert_eq!(lod0.indices.len() / 3, 105_412);
        assert!(
            lod0.indices
                .iter()
                .all(|i| (*i as usize) < lod0.positions.len())
        );
        assert_bounds_match(lod0, &mesh);

        // Every weight references a real bone, and weights are normalised.
        let bones = mesh.bones.len();
        for (js, ws) in lod0.joints.iter().zip(&lod0.weights) {
            for j in js {
                assert!((*j as usize) < bones);
            }
            let sum: f32 = ws.iter().sum();
            assert!((sum - 1.0).abs() < 1e-3, "weights sum to {sum}");
        }
    }

    /// The head package stores its exports in the reverse of the export-map order, so it only
    /// parses through the reverse-offset candidate (`MESH-FORMAT.md`). Asserting the exact end and
    /// counts ties that workaround to the research oracle.
    #[test]
    fn head_mesh_parses_through_the_reverse_order_variant() {
        let Some((mesh, len)) = parse_or_skip(207310, "PC_004_U_CMN_HEAD_001") else {
            return;
        };
        assert_eq!(len, 8_547_785);
        assert_eq!(mesh.bones.len(), 47);
        assert_eq!(mesh.materials.len(), 4);
        assert_eq!(mesh.lods.len(), 4);
        let lod0 = &mesh.lods[0];
        assert_eq!(lod0.positions.len(), 61_068);
        assert_eq!(lod0.indices.len() / 3, 54_770);
        assert_bounds_match(lod0, &mesh);
    }
}
