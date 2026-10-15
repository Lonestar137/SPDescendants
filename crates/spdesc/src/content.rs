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
use bevy::prelude::{Mat4, Mesh, Quat, Transform, Vec3, info};
use bevy::render::mesh::Indices;

use spdesc_data::anim::AnimClip;
use spdesc_data::container::IoStore;
use spdesc_data::mesh::{MeshLod, Section, SkeletalMesh, Transform as UeTransform};
use spdesc_data::property::{Ctx, Names, Natives, Reader, Schema};
use spdesc_data::texture::Texture;
use spdesc_data::zen::{GlobalNames, ZenPackage};

/// One drawable piece of the character: a bind-pose sub-mesh and how to shade it.
pub struct Part {
    pub mesh: Mesh,
    /// Index into [`CharacterAssets::materials`].
    pub material: usize,
    /// Whether the section is skinned to the body skeleton.
    pub skinned: bool,
    /// Which frame the section follows (body, head, or the separate face mesh).
    pub attach: Attach,
}

/// Where a [`Part`] hangs in the character hierarchy.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Attach {
    /// A skinned sub-mesh of the model root (the body).
    Body,
    /// A sub-mesh of the head attach node (the head and hair).
    Head,
    /// A static sub-mesh of the face attach node (the separate face mesh).
    Face,
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
    /// The head mesh's own skeleton (47 bones, including the two braid chains), in Bevy space.
    pub head_skeleton: Skeleton,
    /// The head's attach node: the skeleton bone it follows and its transform relative to that
    /// bone. The head mesh is authored in model space, so the node's bind rotation cancels the
    /// bone's orientation; the head then follows the head bone's motion without twisting.
    pub head_attach: Option<(usize, Transform)>,
    /// The face attach node: the head bone it follows and the transform that puts the face mesh
    /// (authored in body model space) back where it was authored. `None` when the character has no
    /// separate face mesh.
    pub face_attach: Option<(usize, Transform)>,
    /// Vertical (Bevy `+Y`) offset, in metres, that puts the model's lowest point at zero.
    pub feet_offset: f32,
    /// Locomotion clips `[idle, walk, run]` and the bone↔track mapping, when the data resolves.
    pub animation: Option<Animation>,
}

/// Locomotion clips and how their tracks map onto the mesh's bones.
pub struct Animation {
    pub clips: Vec<AnimClip>,
    /// Index of the idle clip within `clips`.
    pub idle: usize,
    /// Index of the walk clip.
    pub walk: usize,
    /// Index of the run clip.
    pub run: usize,
    /// Index of the jump/takeoff clip.
    pub jump: usize,
    /// Index of the airborne loop clip.
    pub air: usize,
    /// Index of the landing clip.
    pub land: usize,
    /// Per clip, for each mesh bone, the track that drives it (clips animate different track sets).
    pub bone_to_track: Vec<Vec<Option<usize>>>,
}

/// A skeleton's bind pose, converted to Bevy space: bone names and parents in order, each bone's
/// local bind transform relative to its parent, and the inverse of each bone's world bind
/// transform (the skinning matrices).
pub struct Skeleton {
    pub names: Vec<String>,
    pub parents: Vec<i32>,
    pub locals: Vec<Transform>,
    pub inverse_bindposes: Vec<Mat4>,
    /// Each bone's bind local transform in the game frame (for posing an animation clip).
    pub bind: Vec<UeTransform>,
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
            .position(|e| !export.is_empty() && e.name == export)
            .or_else(|| {
                package
                    .exports
                    .iter()
                    .position(|e| e.class.ends_with("SkeletalMesh"))
            })
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

    /// A game-space local transform from one decoded animation sample.
    fn from_sample(sample: &spdesc_data::anim::TrackSample) -> Self {
        Self {
            q: sample.rotation.map(f64::from),
            t: sample.translation.map(f64::from),
            s: sample.scale.map(f64::from),
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

/// The decoded PBR maps for one material slot.
#[derive(Default, Clone)]
struct Maps {
    base: Option<Texture>,
    normal: Option<Texture>,
    packed: Option<Texture>,
}

/// The file-name fragments that identify a slot's textures, in the order they are tried. Glass and
/// hair have no decoded maps (they are flat tints), so they want no fragments. Slot names vary a
/// lot between characters (`PartA`, `part_a`, `MI_…_A`), so the trailing token counts too.
fn slot_keywords(slot: &str) -> &'static [&'static str] {
    let s = slot.to_ascii_lowercase();
    let last = s.rsplit('_').next().unwrap_or("");
    if s.contains("glass") || s.contains("hair") || s.contains("fur") {
        &[]
    } else if s.contains("helmet") {
        &["helmet"]
    } else if last == "head" || s.contains("face") {
        &["head", "helmet"]
    } else if s.contains("part_a") || s.contains("parta") || last == "a" {
        &["parta", "part_a"]
    } else if s.contains("part_b") || s.contains("partb") || last == "b" {
        &["partb", "part_b"]
    } else if last == "c" {
        &["partc", "part_c"]
    } else if last == "d" {
        &["partd", "part_d"]
    } else if s.contains("cloth") {
        &["parta", "part_a"]
    } else if s.contains("head") {
        &["head", "helmet"]
    } else if s.contains("skin") || s.contains("body") {
        &["body", "skin"]
    } else {
        &[]
    }
}

/// Load a slot's base colour and its normal and packed siblings from `material_dir`. The base map is
/// any `*_C.uasset` in the folder whose name contains one of `keywords`; the normal and packed maps
/// are the same stem with `_N` / `_P` (BC5 normals and the game's R=AO/G=roughness/B=metallic pack).
/// The base (`_C`) texture in `material_dir` whose name contains one of `keywords`, if any.
fn find_base(
    index: &HashMap<String, u32>,
    material_dir: &str,
    keywords: &[&str],
) -> Option<String> {
    if keywords.is_empty() {
        return None;
    }
    for key in index.keys() {
        if !key.starts_with(material_dir) || !key.ends_with(".uasset") {
            continue;
        }
        let name = key.rsplit('/').next().unwrap_or(key).to_ascii_lowercase();
        if name.ends_with("_c.uasset") && keywords.iter().any(|k| name.contains(k)) {
            return Some(key.clone());
        }
    }
    None
}

fn load_skin_maps(data: &mut DataSource, material_dir: &str, keywords: &[&str]) -> Maps {
    let Some(base) = find_base(&data.index, material_dir, keywords) else {
        return Maps::default();
    };
    let stem = base.trim_end_matches(".uasset");
    let sibling = |suffix: &str| {
        let candidate = format!("{}{suffix}.uasset", &stem[..stem.len() - 2]);
        data.index.contains_key(&candidate).then_some(candidate)
    };
    let normal = sibling("_N");
    let packed = sibling("_P");
    Maps {
        base: data.texture(&base).ok(),
        normal: normal.as_deref().and_then(|p| data.texture(p).ok()),
        packed: packed.as_deref().and_then(|p| data.texture(p).ok()),
    }
}

/// The material a slot name gets: the skin's decoded maps when its folder has them, otherwise a
/// flat tint. Glass and hair are always tints (their materials are not among the decoded maps).
fn material_for(
    data: &mut DataSource,
    material_dir: &str,
    slot: &str,
    cache: &mut HashMap<String, Maps>,
) -> MaterialSpec {
    let keywords = slot_keywords(slot);
    if !keywords.is_empty() {
        let key = format!("{material_dir}|{}", keywords.join(","));
        let maps = cache
            .entry(key)
            .or_insert_with(|| load_skin_maps(data, material_dir, keywords))
            .clone();
        if maps.base.is_some() {
            return MaterialSpec {
                base: maps.base,
                packed: maps.packed,
                normal: maps.normal,
                tint: [1.0, 1.0, 1.0, 1.0],
                // The maps carry the variation; the scalars only multiply them.
                roughness: 1.0,
                metallic: 1.0,
            };
        }
    }
    let s = slot.to_ascii_lowercase();
    let (tint, roughness, metallic) = if s.contains("glass") {
        ([0.45, 0.60, 0.75, 0.35], 0.05, 0.0)
    } else if s.contains("hair") || s.contains("fur") {
        // The braid's colour comes from a hair material this MVP does not decode, so a brown,
        // semi-glossy neutral stands in.
        ([0.26, 0.16, 0.11, 1.0], 0.45, 0.0)
    } else if s.contains("skin") || s.contains("body") {
        // A bare-body slot whose skin texture is not in this skin's folder: a skin tone reads
        // better than the neutral grey the exposed flesh would otherwise get.
        ([0.74, 0.57, 0.47, 1.0], 0.8, 0.0)
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

/// The material for a face-mesh slot. Only the face skin is decoded (from the makeup folder); the
/// eyes, teeth and lash/brow cards are flat tints this MVP does not decode.
fn face_material(
    data: &mut DataSource,
    makeup_dir: &str,
    slot: &str,
    cache: &mut HashMap<String, Maps>,
) -> MaterialSpec {
    let s = slot.to_ascii_lowercase();
    let keywords: &[&str] = if s.contains("face") {
        &["face", "makeup"]
    } else {
        &[]
    };
    if !keywords.is_empty() {
        let key = format!("{makeup_dir}|{}", keywords.join(","));
        let maps = cache
            .entry(key)
            .or_insert_with(|| load_skin_maps(data, makeup_dir, keywords))
            .clone();
        if maps.base.is_some() {
            return MaterialSpec {
                base: maps.base,
                packed: maps.packed,
                normal: maps.normal,
                tint: [1.0, 1.0, 1.0, 1.0],
                roughness: 0.7,
                metallic: 0.0,
            };
        }
    }
    let (tint, roughness) = if s.contains("lash") || s.contains("brow") || s.contains("tear") {
        ([0.05, 0.04, 0.04, 1.0], 0.6)
    } else if s.contains("eye") || s.contains("occ") {
        ([0.80, 0.80, 0.86, 1.0], 0.15)
    } else if s.contains("teeth") {
        ([0.90, 0.90, 0.88, 1.0], 0.3)
    } else {
        ([0.74, 0.57, 0.47, 1.0], 0.7)
    };
    MaterialSpec {
        base: None,
        packed: None,
        normal: None,
        tint,
        roughness,
        metallic: 0.0,
    }
}

/// Load one skin: the body and head mesh package paths, the folder each one's textures live in, and
/// an optional face (makeup) folder. Every section of every LOD 0 becomes a [`Part`]; `feet_offset`
/// places the model on the floor.
pub fn load_character(
    data: &mut DataSource,
    body_path: &str,
    head_path: &str,
    body_material_dir: &str,
    head_material_dir: &str,
    face_material_dir: Option<&str>,
) -> Result<CharacterAssets> {
    let body = data.mesh(body_path, "").context("body mesh")?;
    let head = data.mesh(head_path, "").context("head mesh")?;
    // The head mesh is authored in model space (its twintails reach 1.3 m below its origin), so it
    // cannot simply follow the head bone (whose bind orientation follows the neck and would twist
    // it). Instead an attach node on the head bone cancels that bind orientation, so the head sits
    // upright at the head and still rotates with the bone as it moves.
    let head_index = head_bone(&body);
    let head_world = head_index.map(|i| world_pose(&body, i).to_bevy_mat4());
    let head_attach = head_world.map(|world| {
        let upright = Mat4::from_translation(world.w_axis.truncate());
        let (scale, rotation, translation) =
            (world.inverse() * upright).to_scale_rotation_translation();
        (
            head_index.unwrap(),
            Transform {
                translation,
                rotation,
                scale,
            },
        )
    });

    // The face is a separate mesh authored in *body* model space (its vertices sit at head height),
    // so an attach node carrying the inverse of the head bone's bind transform puts it back exactly
    // where it was authored and keeps it following the head.
    let face = face_mesh_path(body_path).and_then(|path| data.mesh(&path, "").ok());
    let face_attach = head_world.map(|world| {
        let (scale, rotation, translation) = world.inverse().to_scale_rotation_translation();
        (
            head_index.unwrap(),
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
    let mut cache: HashMap<String, Maps> = HashMap::new();
    let mut feet = f32::MAX;
    let mut ignored_feet = f32::MAX;

    let skeleton = build_skeleton(&body);
    build_parts(
        data,
        &body,
        Attach::Body,
        body_material_dir,
        true,
        &mut feet,
        &mut cache,
        &mut materials,
        &mut material_of_slot,
        &mut parts,
    )?;
    build_parts(
        data,
        &head,
        Attach::Head,
        head_material_dir,
        true,
        &mut ignored_feet,
        &mut cache,
        &mut materials,
        &mut material_of_slot,
        &mut parts,
    )?;
    if let Some(face) = &face {
        let dir = face_material_dir
            .map(str::to_string)
            .unwrap_or_else(|| default_face_material_dir(body_path));
        build_parts(
            data,
            face,
            Attach::Face,
            &dir,
            false,
            &mut ignored_feet,
            &mut cache,
            &mut materials,
            &mut material_of_slot,
            &mut parts,
        )?;
    }
    if parts.is_empty() {
        bail!("the character meshes have no renderable sections");
    }
    let feet_offset = if feet.is_finite() { -feet } else { 0.0 };
    let animation = load_animation(data, &skeleton);
    Ok(CharacterAssets {
        parts,
        materials,
        skeleton,
        head_skeleton: build_skeleton(&head),
        head_attach,
        face_attach,
        feet_offset,
        animation,
    })
}

/// The separate face mesh for the character owning `body_path`.
fn face_mesh_path(body_path: &str) -> Option<String> {
    let root = body_path.split("/U/Skel/").next()?;
    let id = root.rsplit('/').next()?;
    Some(format!("{root}/A/Skel/FacePC_{id}_A_Face_000.uasset"))
}

/// The cosmetic (makeup) folder that holds a character's default face texture.
fn default_face_material_dir(body_path: &str) -> String {
    match body_path.split("/U/Skel/").next() {
        Some(root) => format!("{root}/A/Skel/SKIN/Makeup/001"),
        None => String::new(),
    }
}

/// Build one mesh's renderable sections into `parts`, resolving each slot's material.
#[allow(clippy::too_many_arguments)]
fn build_parts(
    data: &mut DataSource,
    mesh: &SkeletalMesh,
    attach: Attach,
    material_dir: &str,
    skinned: bool,
    feet: &mut f32,
    cache: &mut HashMap<String, Maps>,
    materials: &mut Vec<MaterialSpec>,
    material_of_slot: &mut HashMap<String, usize>,
    parts: &mut Vec<Part>,
) -> Result<()> {
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
        let key = format!("{material_dir}|{slot}");
        let material = match material_of_slot.get(&key) {
            Some(i) => *i,
            None => {
                let spec = if attach == Attach::Face {
                    face_material(data, material_dir, &slot, cache)
                } else {
                    material_for(data, material_dir, &slot, cache)
                };
                materials.push(spec);
                let i = materials.len() - 1;
                material_of_slot.insert(key, i);
                i
            }
        };
        parts.push(Part {
            mesh: section_mesh(lod, section, skinned, feet)
                .with_context(|| format!("building section for slot {slot}"))?,
            material,
            skinned,
            attach,
        });
    }
    Ok(())
}

/// Best-effort load of the locomotion clips and the track↔bone mapping. Any failure yields `None`
/// and the character is presented in its bind pose.
fn load_animation(data: &mut DataSource, skeleton: &Skeleton) -> Option<Animation> {
    let order = data.skeleton_order().ok()?;
    let mut load = |path: &str| -> Option<(AnimClip, Vec<Option<usize>>)> {
        let clip = data.clip(path).ok()?;
        let num_tracks = clip.num_tracks as usize;
        let map = data.track_map(path, num_tracks).ok()?;
        let bone_to_track = bone_to_track(skeleton, &order, &map, num_tracks);
        Some((clip, bone_to_track))
    };
    let (idle, idle_map) = load(IDLE_CLIP_PATH)?;
    let (walk, walk_map) = load(WALK_CLIP_PATH).unwrap_or_else(|| (idle.clone(), idle_map.clone()));
    let (run, run_map) = load(RUN_CLIP_PATH).unwrap_or_else(|| (idle.clone(), idle_map.clone()));
    let (jump, jump_map) = load(JUMP_CLIP_PATH).unwrap_or_else(|| (idle.clone(), idle_map.clone()));
    let (air, air_map) = load(AIR_CLIP_PATH).unwrap_or_else(|| (idle.clone(), idle_map.clone()));
    let (land, land_map) = load(LAND_CLIP_PATH).unwrap_or_else(|| (idle.clone(), idle_map.clone()));
    let clips = vec![idle, walk, run, jump, air, land];
    let bone_to_track = vec![idle_map, walk_map, run_map, jump_map, air_map, land_map];
    info!(
        "animation: {} clips, {} bones driven of {} (skeleton has {} bones)",
        clips.len(),
        bone_to_track[0].iter().filter(|m| m.is_some()).count(),
        skeleton.names.len(),
        order.len()
    );
    Some(Animation {
        clips,
        idle: 0,
        walk: 1,
        run: 2,
        jump: 3,
        air: 4,
        land: 5,
        bone_to_track,
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
        bind: mesh.poses.clone(),
    }
}

/// The character skeleton asset. An animation clip's tracks follow this asset's
/// `FReferenceSkeleton` order, so it is the authority that maps a track index to a bone name.
pub const SKELETON_PATH: &str = "/M1/Content/Characters/PCPC_Skeleton.uasset";
/// The looped idle clip.
pub const IDLE_CLIP_PATH: &str =
    "/M1/Content/Characters/PC/Anim/F/CMN/Sequence/IdleF_0LT_Idle.uasset";
/// A forward run clip (the `Move_1200` blend sample used while sprinting).
pub const RUN_CLIP_PATH: &str =
    "/M1/Content/Characters/PC/Anim/F/CMN/Sequence/MoveF_0LT_Move_1200_F.uasset";
/// A forward walk clip.
pub const WALK_CLIP_PATH: &str =
    "/M1/Content/Characters/PC/Anim/F/CMN/Sequence/MoveF_0LT_Move_600_F.uasset";
/// Takeoff, the first part of a jump.
pub const JUMP_CLIP_PATH: &str =
    "/M1/Content/Characters/PC/Anim/F/CMN/Sequence/JumpF_0LT_JumpStart.uasset";
/// The airborne loop that plays after takeoff until touchdown.
pub const AIR_CLIP_PATH: &str =
    "/M1/Content/Characters/PC/Anim/F/CMN/Sequence/JumpF_0LT_JumpLoop.uasset";
/// Touchdown.
pub const LAND_CLIP_PATH: &str =
    "/M1/Content/Characters/PC/Anim/F/CMN/Sequence/JumpF_0LT_JumpLandHard.uasset";

impl DataSource {
    /// The ordered reference-skeleton bone names of the character skeleton (~4930 bones). An
    /// animation's tracks select a subset of these via [`DataSource::track_map`].
    pub fn skeleton_order(&mut self) -> Result<Vec<String>> {
        let chunk = *self
            .index
            .get(SKELETON_PATH)
            .with_context(|| format!("{SKELETON_PATH} is not in the directory index"))?;
        let bytes = self.store.read_chunk(chunk as usize)?;
        let package = ZenPackage::parse(&bytes, &self.global)?;
        let i = package
            .exports
            .iter()
            .position(|e| e.class.contains("Skeleton"))
            .with_context(|| format!("no Skeleton export in {SKELETON_PATH}"))?;
        let names = Names {
            local: &package.local_names,
            global: &self.global.names,
        };
        package
            .export_data_variants(&bytes, i)
            .into_iter()
            .find_map(|data| spdesc_data::skeletal::reference_skeleton_order(data, names))
            .map(|order| order.into_iter().map(|(name, _)| name).collect())
            .with_context(|| format!("the reference skeleton was not found in {SKELETON_PATH}"))
    }

    /// The `CompressedTrackToSkeletonMapTable` of a clip: track `i` animates skeleton bone `map[i]`.
    pub fn track_map(&mut self, path: &str, num_tracks: usize) -> Result<Vec<u32>> {
        let chunk = *self
            .index
            .get(path)
            .with_context(|| format!("{path} is not in the directory index"))?;
        let bytes = self.store.read_chunk(chunk as usize)?;
        let package = ZenPackage::parse(&bytes, &self.global)?;
        let i = package
            .exports
            .iter()
            .position(|e| e.class.contains("AnimSequence"))
            .with_context(|| format!("no AnimSequence export in {path}"))?;
        for data in package.export_data_variants(&bytes, i) {
            if let Some(map) = spdesc_data::anim::track_map(data, num_tracks) {
                return Ok(map);
            }
        }
        bail!("{path} carries no track-to-skeleton map")
    }

    /// Decode one `UAnimSequence` clip to its ACL frames.
    pub fn clip(&mut self, path: &str) -> Result<AnimClip> {
        let chunk = *self
            .index
            .get(path)
            .with_context(|| format!("{path} is not in the directory index"))?;
        let bytes = self.store.read_chunk(chunk as usize)?;
        let package = ZenPackage::parse(&bytes, &self.global)?;
        let i = package
            .exports
            .iter()
            .position(|e| e.class.contains("AnimSequence"))
            .with_context(|| format!("no AnimSequence export in {path}"))?;
        for data in package.export_data_variants(&bytes, i) {
            if let Some(blob) = spdesc_data::anim::find_blob(data) {
                return spdesc_data::anim::decode(blob).context("decoding an ACL clip");
            }
        }
        bail!("{path} carries no ACL compressed_tracks blob")
    }
}

/// Map each mesh bone to the animation track that drives it. `map[track]` is the skeleton bone the
/// track animates; `order` names those bones, so a track reaches a mesh bone by name.
pub fn bone_to_track(
    skeleton: &Skeleton,
    order: &[String],
    map: &[u32],
    num_tracks: usize,
) -> Vec<Option<usize>> {
    let mut out = vec![None; skeleton.names.len()];
    for (track, bone_index) in map.iter().enumerate().take(num_tracks) {
        let Some(name) = order.get(*bone_index as usize) else {
            continue;
        };
        if let Some(bone) = skeleton.names.iter().position(|n| n == name) {
            out[bone] = Some(track);
        }
    }
    out
}

/// Pose every mesh bone for one frame of `clip`: bones with a mapped track take the clip's local
/// transform, the rest keep their bind pose. The game-space locals are composed up the mesh
/// hierarchy and converted to Bevy bone-local transforms.
pub fn pose_bone_locals(
    skeleton: &Skeleton,
    clip: &AnimClip,
    bone_to_track: &[Option<usize>],
    sample: usize,
) -> Vec<Transform> {
    let n = skeleton.names.len();
    let num_tracks = clip.num_tracks as usize;
    let frame = &clip.samples[sample.min(clip.num_samples as usize - 1) * num_tracks..];
    let mut local_game = Vec::with_capacity(n);
    for bone in 0..n {
        let posed = match bone_to_track.get(bone).copied().flatten() {
            Some(track) if track < frame.len() => Some(Xform::from_sample(&frame[track])),
            _ => None,
        };
        local_game.push(posed.unwrap_or_else(|| Xform::local(&skeleton.bind[bone])));
    }
    let mut world_game = vec![Xform::IDENTITY; n];
    for bone in 0..n {
        let parent = skeleton.parents[bone];
        world_game[bone] = if parent >= 0 {
            world_game[parent as usize].compose(&local_game[bone])
        } else {
            local_game[bone]
        };
    }
    let world_bevy: Vec<Mat4> = world_game.iter().map(|x| x.to_bevy_mat4()).collect();
    (0..n)
        .map(|bone| {
            let parent = skeleton.parents[bone];
            let local = if parent >= 0 {
                world_bevy[parent as usize].inverse() * world_bevy[bone]
            } else {
                world_bevy[bone]
            };
            let (scale, rotation, translation) = local.to_scale_rotation_translation();
            Transform {
                translation,
                rotation,
                scale,
            }
        })
        .collect()
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
        let body =
            "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/001PC_004_U_CMN_BODY_001.uasset";
        let head =
            "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/001PC_004_U_CMN_HEAD_001.uasset";
        let material = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/001";
        Some(load_character(&mut data, body, head, material, material, None).expect("load a skin"))
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

    /// Posing the idle clip drives the mesh bones and changes them frame to frame.
    #[test]
    fn bunny_animation_poses_and_moves_its_bones() {
        let Some(assets) = build_or_skip() else {
            return;
        };
        let animation = assets.animation.as_ref().expect("the idle clip loads");
        let map = &animation.bone_to_track[animation.idle];
        let driven = map.iter().filter(|t| t.is_some()).count();
        assert!(driven > 100, "only {driven} bones are driven by the clip");
        let clip = &animation.clips[animation.idle];
        let a = pose_bone_locals(&assets.skeleton, clip, map, 0);
        let b = pose_bone_locals(&assets.skeleton, clip, map, clip.num_samples as usize / 2);
        let moved = a
            .iter()
            .zip(&b)
            .filter(|(x, y)| {
                x.translation.distance(y.translation) > 1e-3
                    || x.rotation.angle_between(y.rotation) > 1e-3
            })
            .count();
        assert!(
            moved > 3,
            "the idle changes only {moved} bone poses over half a loop"
        );
    }

    /// The jump/fall/land clips load (not falling back to the idle).
    #[test]
    fn jump_and_land_clips_load() {
        let Some(assets) = build_or_skip() else {
            return;
        };
        let a = assets.animation.as_ref().expect("the clips load");
        assert_eq!(a.clips.len(), 6);
        assert_eq!(a.clips[a.jump].num_samples, 31, "JumpStart");
        assert_eq!(a.clips[a.air].num_samples, 61, "JumpLoop");
        assert_eq!(a.clips[a.land].num_samples, 44, "JumpLandHard");
    }

    /// The head skeleton carries the braid chains the hair dynamics find by name; if the data ever
    /// stops naming them `Bn_Hair*_Parent`, the physics would silently do nothing.
    #[test]
    fn head_skeleton_has_the_braid_chains() {
        let Some(assets) = build_or_skip() else {
            return;
        };
        let names = &assets.head_skeleton.names;
        assert!(
            names.iter().any(|n| n == "Bn_Hair30_Parent"),
            "the head skeleton carries the Bn_Hair30 braid"
        );
        assert!(
            names.iter().any(|n| n == "Bn_Hair34_Parent"),
            "the head skeleton carries the Bn_Hair34 braid"
        );
    }

    /// The catalog enumerates characters and skins from the real index, and every skin names a
    /// body mesh and a material folder.
    #[test]
    fn catalog_has_characters_and_skins() {
        let Some(paks) = spdesc_data::paks_dir() else {
            return;
        };
        let Some(key) = spdesc_data::key::aes_key() else {
            return;
        };
        let Ok(schema) = spdesc_data::mapping::load() else {
            return;
        };
        let data = DataSource::open(&paks, key, schema).expect("open data source");
        let catalog = crate::catalog::build(&data.index, &crate::catalog::load_names());
        for character in &catalog {
            eprintln!(
                "  {}/{} : {} skins",
                character.id,
                character.name,
                character.skins.len()
            );
            for skin in &character.skins {
                eprintln!(
                    "      {} body={:?} head={:?} material={:?}",
                    skin.label,
                    skin.body.rsplit('/').next().unwrap(),
                    skin.head.rsplit('/').next().unwrap(),
                    skin.material_dir
                        .rmatch_indices("SKIN/")
                        .next()
                        .map(|(i, _)| &skin.material_dir[i + 5..])
                );
            }
        }
        assert!(!catalog.is_empty(), "the catalog is empty");
        assert!(
            catalog.iter().any(|c| c.skins.len() > 1),
            "a character with several skins is expected"
        );
    }

    /// A pure conversion check that does not need data: game `(x, y, z)` cm maps to Bevy
    /// `(x, z, -y)` m.
    #[test]
    fn game_space_converts_to_bevy_space() {
        assert_eq!(to_bevy([100.0, 200.0, 300.0]), [1.0, 3.0, -2.0]);
        assert_eq!(dir_to_bevy([0.0, 1.0, 0.0]), [0.0, 0.0, -1.0]);
    }

    /// Slot names pick the texture fragments to look for, and glass/hair never resolve to decoded
    /// maps (they are flat tints).
    #[test]
    fn slots_choose_their_texture_keywords() {
        assert_eq!(
            slot_keywords("PC_004_U_CMN_BODY_001_PartA"),
            ["parta", "part_a"]
        );
        assert_eq!(slot_keywords("PC_001_A_MF_001_part_b"), ["partb", "part_b"]);
        assert_eq!(slot_keywords("PC_016_U_CMN_001_A"), ["parta", "part_a"]);
        assert_eq!(slot_keywords("PC_016_U_CMN_001_B"), ["partb", "part_b"]);
        assert_eq!(
            slot_keywords("PC_004_U_CMN_BODY_001_Skin"),
            ["body", "skin"]
        );
        assert_eq!(slot_keywords("PC_001_A_MF_001_Head"), ["head", "helmet"]);
        assert_eq!(slot_keywords("PC_016_U_CMN_001_HEAD"), ["head", "helmet"]);
        assert_eq!(slot_keywords("PC_004_U_CMN_HEAD_001_Helmet"), ["helmet"]);
        assert!(slot_keywords("PC_004_U_CMN_HEAD_001_Hair").is_empty());
        assert!(slot_keywords("PC_004_U_CMN_BODY_001_GlassA").is_empty());
        assert!(slot_keywords("something_else").is_empty());
    }
}
