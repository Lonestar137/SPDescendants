//! Building Bevy assets from the player's read-only `GAME_DATA`.
//!
//! The app owns no asset formats: `spdesc-data` parses the container, the Zen package, the
//! `USkeletalMesh` render data, and the `UTexture2D` pixels. This module only converts those
//! decoded values into Bevy meshes and images, at the presentation boundary:
//!
//! * the game is left-handed with `+X` forward, `+Y` right, `+Z` up and centimetres;
//! * Bevy is right-handed `+Y` up, and this game plays at metre scale, so a vertex
//!   `(x, y, z)` becomes `(x, z, -y) * 0.01`.
//!
//! Damaged or absent data is a fallback here, never a panic: the caller gets a `Result` and the
//! scene still comes up.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::{Mat4, Mesh, Quat, Transform, Vec3, warn};
use bevy::render::mesh::Indices;

use spdesc_data::container::IoStore;
use spdesc_data::mesh::{MeshLod, Section, SkeletalMesh, Transform as UeTransform};
use spdesc_data::property::{Ctx, Names, Natives, Reader, Schema};
use spdesc_data::texture::Texture;
use spdesc_data::zen::{GlobalNames, ZenPackage};

/// The Ultimate Bunny body mesh package path and its mesh export name.
pub const BODY_PATH: &str =
    "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/001PC_004_U_CMN_BODY_001.uasset";
const BODY_EXPORT: &str = "PC_004_U_CMN_BODY_001";
/// The Ultimate Bunny head mesh package path and its mesh export name.
pub const HEAD_PATH: &str =
    "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/001PC_004_U_CMN_HEAD_001.uasset";
const HEAD_EXPORT: &str = "PC_004_U_CMN_HEAD_001";
/// The body (skin) base-colour texture.
pub const BODY_TEXTURE_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Body_C.uasset";
/// The PartA base-colour texture.
pub const PARTA_TEXTURE_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_PartA_C.uasset";
/// The PartB base-colour texture.
pub const PARTB_TEXTURE_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_PartB_C.uasset";
/// The helmet base-colour texture.
pub const HELMET_TEXTURE_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Helmet_C.uasset";
/// The PartA normal map (BC5).
const PARTA_NORMAL_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_PartA_N.uasset";
/// The PartA packed map (R=AO, G=roughness, B=metallic).
const PARTA_PACKED_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_PartA_P.uasset";
/// The PartB normal map (BC5).
const PARTB_NORMAL_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_PartB_N.uasset";
/// The PartB packed map.
const PARTB_PACKED_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_PartB_P.uasset";
/// The helmet normal map (BC5).
const HELMET_NORMAL_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Helmet_N.uasset";
/// The helmet packed map.
const HELMET_PACKED_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Helmet_P.uasset";

/// One drawable piece of the character: a bind-pose sub-mesh and how to shade it.
pub struct Part {
    pub mesh: Mesh,
    /// Index into [`CharacterAssets::materials`].
    pub material: usize,
    /// Whether the section is skinned to the body skeleton.
    pub skinned: bool,
    /// Whether the section hangs off the head attach node (the head) rather than the player.
    pub under_head: bool,
}

/// The PBR maps and scalars for one material slot. `base`/`normal`/`packed` are the decoded
/// textures when the slot has them; `tint` is the flat colour otherwise.
pub struct MaterialSpec {
    pub base: Option<Texture>,
    /// The game's packed map: R = ambient occlusion, G = roughness, B = metallic.
    pub packed: Option<Texture>,
    pub normal: Option<Texture>,
    pub tint: [f32; 4],
    /// Roughness scalar, multiplied by the packed map's G when present.
    pub roughness: f32,
    /// Metallic scalar, multiplied by the packed map's B when present.
    pub metallic: f32,
}

/// A loaded character: its parts, their materials, and where its feet sit in Bevy space.
pub struct CharacterAssets {
    pub parts: Vec<Part>,
    pub materials: Vec<MaterialSpec>,
    /// The body skeleton, in Bevy space, for skinning and procedural posing.
    pub skeleton: Skeleton,
    /// The head's attach node: the skeleton bone it follows and its transform relative to that
    /// bone. The head mesh is authored in model space, so the node's bind rotation cancels the
    /// bone's orientation; the head then follows the head bone's motion without twisting.
    pub head_attach: Option<(usize, Transform)>,
    /// Vertical (Bevy `+Y`) offset, in metres, that puts the model's lowest point at zero.
    pub feet_offset: f32,
}

/// A skeleton's bind pose, converted to Bevy space: bone names and parents in order, each bone's
/// local bind transform relative to its parent, and the inverse of each bone's world bind
/// transform (the skinning matrices).
pub struct Skeleton {
    pub names: Vec<String>,
    pub parents: Vec<i32>,
    pub locals: Vec<Transform>,
    pub inverse_bindposes: Vec<Mat4>,
}

/// The container, its directory index, and the global name table: everything the per-asset
/// loaders share.
pub struct DataSource {
    pub store: IoStore,
    pub index: HashMap<String, u32>,
    pub global: GlobalNames,
    pub schema: Schema,
    pub natives: Natives,
}

impl DataSource {
    /// Open the container for `paks` with `key`. Reads the directory index, the global name table,
    /// and the property mapping once; every asset load reuses them.
    pub fn open(paks: &std::path::Path, key: [u8; 32], schema: Schema) -> Result<Self> {
        let store = IoStore::open(paks, key)?;
        let index = store.directory_index()?;
        let global = GlobalNames::read(paks)?;
        let mut natives = Natives::new();
        spdesc_data::skeletal::register_natives(&mut natives);
        Ok(Self {
            store,
            index,
            global,
            schema,
            natives,
        })
    }

    /// Parse one `USkeletalMesh` export to its full render data.
    fn mesh(&mut self, path: &str, export: &str) -> Result<SkeletalMesh> {
        let chunk = *self
            .index
            .get(path)
            .with_context(|| format!("{path} is not in the directory index"))?;
        let bytes = self.store.read_chunk(chunk as usize)?;
        let package = ZenPackage::parse(&bytes, &self.global)?;
        let i = package
            .exports
            .iter()
            .position(|e| e.name == export)
            .with_context(|| format!("{export} export missing from {path}"))?;
        let variants = package.export_data_variants(&bytes, i);
        if variants.is_empty() {
            bail!("{export} cooked range is out of bounds in {path}");
        }
        // The Ultimate Bunny head stores its exports in reverse physical order, so the first
        // offset candidate may not be the mesh; parse each until one consumes exactly.
        let mut last: Option<anyhow::Error> = None;
        for data in variants {
            let ctx = Ctx {
                schema: &self.schema,
                names: Names {
                    local: &package.local_names,
                    global: &self.global.names,
                },
                natives: &self.natives,
            };
            let mut reader = Reader::new(data);
            match spdesc_data::mesh::parse(&mut reader, &ctx) {
                Ok(mesh) => return Ok(mesh),
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("no export range parsed")))
            .with_context(|| format!("parsing {export} in {path}"))
    }

    /// Decode one `UTexture2D` to mip 0 RGBA.
    fn texture(&mut self, path: &str) -> Result<Texture> {
        spdesc_data::texture::load(
            &mut self.store,
            &self.index,
            &self.global,
            &self.schema,
            &self.natives,
            path,
        )
    }
}

/// Convert a game-space position (cm, `+Z` up) to Bevy space (m, `+Y` up).
fn to_bevy(v: [f32; 3]) -> [f32; 3] {
    [v[0] * 0.01, v[2] * 0.01, -v[1] * 0.01]
}

/// Convert a game-space direction (unit) to Bevy space.
fn dir_to_bevy(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2], -v[1]]
}

/// A UE `FTransform` (quaternion `xyzw`, translation, scale), used only to compose a skeleton's
/// bind pose so a modular mesh can be attached to the bone it was authored against.
#[derive(Clone, Copy)]
struct Xform {
    q: [f64; 4],
    t: [f64; 3],
    s: [f64; 3],
}

impl Xform {
    const IDENTITY: Xform = Xform {
        q: [0.0, 0.0, 0.0, 1.0],
        t: [0.0; 3],
        s: [1.0; 3],
    };

    fn local(pose: &UeTransform) -> Self {
        Self {
            q: [
                f64::from(pose.quat[0]),
                f64::from(pose.quat[1]),
                f64::from(pose.quat[2]),
                f64::from(pose.quat[3]),
            ],
            t: pose.translation.map(f64::from),
            s: pose.scale.map(f64::from),
        }
    }

    /// `self * child`: apply `child` in `self`'s frame.
    fn compose(&self, child: &Xform) -> Xform {
        Xform {
            q: quat_mul(self.q, child.q),
            t: self.apply(child.t),
            s: [
                self.s[0] * child.s[0],
                self.s[1] * child.s[1],
                self.s[2] * child.s[2],
            ],
        }
    }

    fn apply(&self, v: [f64; 3]) -> [f64; 3] {
        let scaled = [v[0] * self.s[0], v[1] * self.s[1], v[2] * self.s[2]];
        let r = quat_rotate(self.q, scaled);
        [r[0] + self.t[0], r[1] + self.t[1], r[2] + self.t[2]]
    }

    /// The Bevy-space matrix for this transform: the game→Bevy basis change `R` conjugated about
    /// the transform, with the translation already scaled to metres. This is the same mapping
    /// [`to_bevy`] applies to a point.
    fn to_bevy_mat4(self) -> Mat4 {
        let local = Mat4::from_scale_rotation_translation(
            Vec3::new(self.s[0] as f32, self.s[1] as f32, self.s[2] as f32),
            Quat::from_xyzw(
                self.q[0] as f32,
                self.q[1] as f32,
                self.q[2] as f32,
                self.q[3] as f32,
            ),
            Vec3::new(
                (self.t[0] * 0.01) as f32,
                (self.t[1] * 0.01) as f32,
                (self.t[2] * 0.01) as f32,
            ),
        );
        let basis = Mat4::from_quat(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
        basis * local * basis.transpose()
    }
}

fn quat_mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn quat_rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let tx = 2.0 * (y * v[2] - z * v[1]);
    let ty = 2.0 * (z * v[0] - x * v[2]);
    let tz = 2.0 * (x * v[1] - y * v[0]);
    [
        v[0] + w * tx + (y * tz - z * ty),
        v[1] + w * ty + (z * tx - x * tz),
        v[2] + w * tz + (x * ty - y * tx),
    ]
}

/// The world (component-space) bind transform of bone `index`: its local pose composed up the
/// parent chain from the root.
fn world_pose(mesh: &SkeletalMesh, index: usize) -> Xform {
    let mut chain = Vec::new();
    let mut i = index as i32;
    while i >= 0 {
        chain.push(i as usize);
        i = mesh.bones[i as usize].parent;
    }
    let mut world = Xform::IDENTITY;
    for &bone in chain.iter().rev() {
        world = world.compose(&Xform::local(&mesh.poses[bone]));
    }
    world
}

/// The body bone a modular head mesh is authored against: the head bone, not its socket.
fn head_bone(mesh: &SkeletalMesh) -> Option<usize> {
    mesh.bones
        .iter()
        .position(|b| b.name.ends_with("Head") && !b.name.contains("Socket"))
        .or_else(|| mesh.bones.iter().position(|b| b.name.contains("Head")))
}

/// Build a Bevy [`Mesh`] for one render section of `lod`, remapping its referenced vertices into a
/// compact local range. `attach` places a modular mesh (the head) authored in another bone's frame
/// into the body's component space.
fn section_mesh(lod: &MeshLod, section: &Section, skinned: bool, feet: &mut f32) -> Result<Mesh> {
    let base = section.base_index as usize;
    let triangles = section.num_triangles.max(0) as usize;
    let index_count = triangles
        .checked_mul(3)
        .context("section triangle count overflows")?;
    if base + index_count > lod.indices.len() {
        bail!(
            "section index range {base}..{} out of bounds",
            base + index_count
        );
    }

    let mut remap: HashMap<usize, u32> = HashMap::new();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut joints: Vec<[u16; 4]> = Vec::new();
    let mut weights: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::with_capacity(index_count);
    for k in 0..index_count {
        let global = lod.indices[base + k] as usize;
        let local = match remap.get(&global) {
            Some(l) => *l,
            None => {
                let p = *lod
                    .positions
                    .get(global)
                    .with_context(|| format!("index {global} outside the vertex buffer"))?;
                let n = lod.normals.get(global).copied().unwrap_or([0.0, 0.0, 1.0]);
                let uv = lod.uvs.get(global).copied().unwrap_or([0.0, 0.0]);
                let b = to_bevy(p);
                *feet = feet.min(b[1]);
                let l = positions.len() as u32;
                remap.insert(global, l);
                positions.push(b);
                normals.push(dir_to_bevy(n));
                uvs.push(uv);
                joints.push(lod.joints.get(global).copied().unwrap_or([0; 4]));
                weights.push(lod.weights.get(global).copied().unwrap_or([0.0; 4]));
                l
            }
        };
        indices.push(local);
    }
    // Unreal's front faces are clockwise; Bevy culls counter-clockwise back faces. The
    // game-space → Bevy-space change is a pure rotation, so the winding must be reversed here or
    // every front face is culled and the model renders as a hollow, see-through shell.
    for triangle in indices.as_chunks_mut::<3>().0 {
        triangle.swap(0, 2);
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    // A static attach (the head) must not carry joint data: a mesh with joint attributes but no
    // `SkinnedMesh` is put on the skinned pipeline and gets the wrong bind group.
    if skinned {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(joints),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_WEIGHT,
            VertexAttributeValues::Float32x4(weights),
        );
    }
    mesh.insert_indices(Indices::U32(indices));
    Ok(mesh)
}

/// The decoded PBR maps for one material slot. Each slot's maps are consumed once, by the first
/// section that names it.
#[derive(Default)]
struct Maps {
    base: Option<Texture>,
    normal: Option<Texture>,
    packed: Option<Texture>,
}

/// Decode a slot's base colour, and its normal and packed maps when it has them.
fn load_maps(
    data: &mut DataSource,
    base: &str,
    normal: Option<&str>,
    packed: Option<&str>,
) -> Maps {
    Maps {
        base: data.texture(base).ok(),
        normal: normal.and_then(|p| data.texture(p).ok()),
        packed: packed.and_then(|p| data.texture(p).ok()),
    }
}

/// The material a slot name gets, removing that slot's maps from `slots` on first use. Slots are
/// matched case-insensitively on a substring; glass and hair are tints (their materials are not
/// among the decoded maps).
fn material_for(slot: &str, slots: &mut HashMap<&'static str, Maps>) -> MaterialSpec {
    let s = slot.to_ascii_lowercase();
    let key = if s.contains("glass") || s.contains("hair") {
        None
    } else if s.contains("helmet") {
        Some("helmet")
    } else if s.contains("partb") {
        Some("partb")
    } else if s.contains("part") {
        Some("parta")
    } else if s.contains("skin") || s.contains("body") {
        Some("skin")
    } else {
        None
    };
    if let Some(key) = key
        && let Some(m) = slots.remove(key)
    {
        return MaterialSpec {
            base: m.base,
            packed: m.packed,
            normal: m.normal,
            tint: [1.0, 1.0, 1.0, 1.0],
            // The maps carry the variation; the scalars only multiply them.
            roughness: 1.0,
            metallic: 1.0,
        };
    }
    let (tint, roughness, metallic) = if s.contains("glass") {
        ([0.45, 0.60, 0.75, 0.35], 0.05, 0.0)
    } else if s.contains("hair") {
        // Bunny's hair is a brown braid; its colour comes from a hair material this MVP does not
        // decode, so a brown, semi-glossy neutral stands in.
        ([0.26, 0.16, 0.11, 1.0], 0.45, 0.0)
    } else {
        ([0.6, 0.55, 0.5, 1.0], 0.6, 0.0)
    };
    MaterialSpec {
        base: None,
        packed: None,
        normal: None,
        tint,
        roughness,
        metallic,
    }
}

/// Load the Ultimate Bunny body and head, decoding the two body textures as well. Every section of
/// every LOD 0 becomes a [`Part`]; the returned `feet_offset` places the model on the floor.
pub fn load_bunny(data: &mut DataSource) -> Result<CharacterAssets> {
    let body = data.mesh(BODY_PATH, BODY_EXPORT).context("body mesh")?;
    let head = data.mesh(HEAD_PATH, HEAD_EXPORT).context("head mesh")?;
    let mut slots: HashMap<&'static str, Maps> = HashMap::new();
    slots.insert("skin", load_maps(data, BODY_TEXTURE_PATH, None, None));
    slots.insert(
        "parta",
        load_maps(
            data,
            PARTA_TEXTURE_PATH,
            Some(PARTA_NORMAL_PATH),
            Some(PARTA_PACKED_PATH),
        ),
    );
    slots.insert(
        "partb",
        load_maps(
            data,
            PARTB_TEXTURE_PATH,
            Some(PARTB_NORMAL_PATH),
            Some(PARTB_PACKED_PATH),
        ),
    );
    slots.insert(
        "helmet",
        load_maps(
            data,
            HELMET_TEXTURE_PATH,
            Some(HELMET_NORMAL_PATH),
            Some(HELMET_PACKED_PATH),
        ),
    );
    if slots["skin"].base.is_none() {
        warn!("body texture failed to decode; using flat colours");
    }

    // The head mesh is authored in model space (its twintails reach 1.3 m below its origin), so it
    // cannot simply follow the head bone (whose bind orientation follows the neck and would twist
    // it). Instead an attach node on the head bone cancels that bind orientation, so the head sits
    // upright at the head and still rotates with the bone as it moves.
    let head_attach = head_bone(&body).map(|index| {
        let world = world_pose(&body, index).to_bevy_mat4();
        let upright = Mat4::from_translation(world.w_axis.truncate());
        let (scale, rotation, translation) =
            (world.inverse() * upright).to_scale_rotation_translation();
        (
            index,
            Transform {
                translation,
                rotation,
                scale,
            },
        )
    });

    let mut materials: Vec<MaterialSpec> = Vec::new();
    let mut material_of_slot: HashMap<String, usize> = HashMap::new();
    let mut parts: Vec<Part> = Vec::new();
    let mut feet = f32::MAX;

    let skeleton = build_skeleton(&body);
    for (mesh, skinned) in [(&body, true), (&head, false)] {
        let lod = mesh.lods.first().context("mesh has no LOD 0")?;
        for section in &lod.sections {
            if section.disabled {
                continue;
            }
            let slot = mesh
                .materials
                .get(section.material_index.max(0) as usize)
                .map(|m| m.slot_name.clone())
                .unwrap_or_else(|| format!("slot{}", section.material_index));
            let material = match material_of_slot.get(&slot) {
                Some(i) => *i,
                None => {
                    let material_spec = material_for(&slot, &mut slots);
                    materials.push(material_spec);
                    let i = materials.len() - 1;
                    material_of_slot.insert(slot.clone(), i);
                    i
                }
            };
            // Only the body (in component space) sets the floor; the head's local space would
            // skew it.
            let mut ignored_feet = f32::MAX;
            let feet_ref = if skinned {
                &mut feet
            } else {
                &mut ignored_feet
            };
            parts.push(Part {
                mesh: section_mesh(lod, section, skinned, feet_ref)
                    .with_context(|| format!("building section for slot {slot}"))?,
                material,
                skinned,
                under_head: !skinned,
            });
        }
    }
    if parts.is_empty() {
        bail!("the character meshes have no renderable sections");
    }
    let feet_offset = if feet.is_finite() { -feet } else { 0.0 };
    Ok(CharacterAssets {
        parts,
        materials,
        skeleton,
        head_attach,
        feet_offset,
    })
}

/// The body skeleton in Bevy space: bone names and parents in order, each bone's local bind
/// transform relative to its parent, and the inverse of each bone's world bind transform.
fn build_skeleton(mesh: &SkeletalMesh) -> Skeleton {
    let n = mesh.bones.len();
    let world: Vec<Mat4> = (0..n).map(|i| world_pose(mesh, i).to_bevy_mat4()).collect();
    let inverse_bindposes: Vec<Mat4> = world.iter().map(Mat4::inverse).collect();
    let locals: Vec<Transform> = (0..n)
        .map(|i| {
            let parent = mesh.bones[i].parent;
            let local = if parent >= 0 {
                world[parent as usize].inverse() * world[i]
            } else {
                world[i]
            };
            let (scale, rotation, translation) = local.to_scale_rotation_translation();
            Transform {
                translation,
                rotation,
                scale,
            }
        })
        .collect();
    Skeleton {
        names: mesh.bones.iter().map(|b| b.name.clone()).collect(),
        parents: mesh.bones.iter().map(|b| b.parent).collect(),
        locals,
        inverse_bindposes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the character from this machine's data, if the folder, key, and mapping resolve.
    fn build_or_skip() -> Option<CharacterAssets> {
        let Some(paks) = spdesc_data::paks_dir() else {
            spdesc_game_core::skipped("no game data found", &spdesc_game_core::candidates());
            return None;
        };
        let Some(key) = spdesc_data::key::aes_key() else {
            spdesc_game_core::skipped(
                "no container key (spdesc-config/keys.json)",
                &spdesc_data::key::config_root()
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
            return None;
        };
        let Ok(schema) = spdesc_data::mapping::load() else {
            spdesc_game_core::skipped(
                "no property mapping (spdesc-config/mappings)",
                &spdesc_data::mapping::mapping_path()
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
            return None;
        };
        let mut data = DataSource::open(&paks, key, schema).expect("open data source");
        Some(load_bunny(&mut data).expect("load the Ultimate Bunny"))
    }

    /// The app's content path builds the body and head meshes and decodes their textures from the
    /// player's folder. This is the data-gated proof that startup has real geometry, not fallback.
    #[test]
    fn bunny_has_geometry_and_textures_from_game_data() {
        let Some(assets) = build_or_skip() else {
            return;
        };
        assert!(
            assets.parts.len() >= 9,
            "body (5) + head (4) sections expected, got {}",
            assets.parts.len()
        );
        assert!(
            assets.materials.iter().any(|m| m.base.is_some()),
            "at least one decoded game texture must shade a section"
        );
        assert!(
            assets
                .materials
                .iter()
                .any(|m| m.packed.is_some() && m.normal.is_some()),
            "the armour slots must carry their packed and normal maps"
        );
        assert!(
            assets.feet_offset.is_finite() && assets.feet_offset.abs() < 5.0,
            "feet offset {} m is implausible",
            assets.feet_offset
        );
    }

    /// A pure conversion check that does not need data: game `(x, y, z)` cm maps to Bevy
    /// `(x, z, -y)` m.
    #[test]
    fn game_space_converts_to_bevy_space() {
        assert_eq!(to_bevy([100.0, 200.0, 300.0]), [1.0, 3.0, -2.0]);
        assert_eq!(dir_to_bevy([0.0, 1.0, 0.0]), [0.0, 0.0, -1.0]);
    }

    #[test]
    fn slots_choose_their_material() {
        let tex = || Texture {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        };
        let maps = || Maps {
            base: Some(tex()),
            normal: None,
            packed: None,
        };
        let mut slots: HashMap<&'static str, Maps> = HashMap::new();
        slots.insert("skin", maps());
        slots.insert("parta", maps());
        slots.insert("partb", maps());
        slots.insert("helmet", maps());
        assert!(material_for("Skin", &mut slots).base.is_some());
        assert!(material_for("PartA", &mut slots).base.is_some());
        assert!(material_for("PartB", &mut slots).base.is_some());
        assert!(material_for("Helmet", &mut slots).base.is_some());
        assert!(material_for("Hair", &mut slots).base.is_none());
        assert!(material_for("GlassA", &mut slots).base.is_none());
    }
}
