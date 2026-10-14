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
use bevy::prelude::{Mesh, info, warn};
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
/// The body (skin) texture.
pub const BODY_TEXTURE_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Body_C.uasset";
/// The PartA texture.
pub const PARTA_TEXTURE_PATH: &str = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/001/TexturePC_004_U_CMN_001_PartA_C.uasset";

/// One drawable piece of the character: a bind-pose sub-mesh and how to shade it.
pub struct Part {
    pub mesh: Mesh,
    /// Index into [`CharacterAssets::materials`].
    pub material: usize,
}

/// The kit needed to shade the character.
pub enum Shading {
    /// A decoded game texture.
    Texture(Texture),
    /// A flat colour, used when a slot's texture is not among the decoded ones.
    Solid([f32; 4]),
}

/// A loaded character: its parts, their shading, and where its feet sit in Bevy space.
pub struct CharacterAssets {
    pub parts: Vec<Part>,
    pub materials: Vec<Shading>,
    /// Vertical (Bevy `+Y`) offset, in metres, that puts the model's lowest point at zero.
    pub feet_offset: f32,
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

    fn rotate_dir(&self, v: [f32; 3]) -> [f32; 3] {
        let r = quat_rotate(self.q, [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]);
        [r[0] as f32, r[1] as f32, r[2] as f32]
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
fn section_mesh(
    lod: &MeshLod,
    section: &Section,
    attach: Option<Xform>,
    feet: &mut f32,
) -> Result<Mesh> {
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
                let mut n = lod.normals.get(global).copied().unwrap_or([0.0, 0.0, 1.0]);
                let uv = lod.uvs.get(global).copied().unwrap_or([0.0, 0.0]);
                let p = match attach {
                    Some(a) => {
                        let t = a.apply([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]);
                        [t[0] as f32, t[1] as f32, t[2] as f32]
                    }
                    None => p,
                };
                if let Some(a) = attach {
                    n = a.rotate_dir(n);
                }
                let b = to_bevy(p);
                *feet = feet.min(b[1]);
                let l = positions.len() as u32;
                remap.insert(global, l);
                positions.push(b);
                normals.push(dir_to_bevy(n));
                uvs.push(uv);
                l
            }
        };
        indices.push(local);
    }

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    Ok(mesh)
}

/// The shading each material slot gets. The body's `Skin` uses the body texture and its `PartA`
/// the PartA texture; every other slot (hair, helmet, and the glass the MVP does not decode) is a
/// flat colour. Slots are matched case-insensitively on a substring, and a texture is taken at
/// most once.
fn shading_for(slot: &str, body: &mut Option<Texture>, part_a: &mut Option<Texture>) -> Shading {
    let s = slot.to_ascii_lowercase();
    if (s.contains("skin") || s.contains("body"))
        && let Some(t) = body.take()
    {
        return Shading::Texture(t);
    }
    if s.contains("part")
        && let Some(t) = part_a.take()
    {
        return Shading::Texture(t);
    }
    // A muted colour for hair, helmet, and the glass parts.
    let color = if s.contains("glass") {
        [0.55, 0.65, 0.8, 1.0]
    } else if s.contains("hair") || s.contains("helmet") {
        [0.12, 0.12, 0.14, 1.0]
    } else {
        [0.6, 0.55, 0.5, 1.0]
    };
    Shading::Solid(color)
}

/// Load the Ultimate Bunny body and head, decoding the two body textures as well. Every section of
/// every LOD 0 becomes a [`Part`]; the returned `feet_offset` places the model on the floor.
pub fn load_bunny(data: &mut DataSource) -> Result<CharacterAssets> {
    let body = data.mesh(BODY_PATH, BODY_EXPORT).context("body mesh")?;
    let head = data.mesh(HEAD_PATH, HEAD_EXPORT).context("head mesh")?;
    let body_tex = data.texture(BODY_TEXTURE_PATH).ok();
    let part_a_tex = data.texture(PARTA_TEXTURE_PATH).ok();
    if body_tex.is_none() {
        warn!("body texture failed to decode; using flat colours");
    }

    // The head is authored in the head bone's frame (its twintails reach 1.3 m below its origin),
    // so it is placed with the body's head-bone bind transform.
    let attach = head_bone(&body).map(|i| world_pose(&body, i));
    if let Some(a) = attach {
        info!(
            "head bone bind position: [{:.1}, {:.1}, {:.1}] cm",
            a.t[0], a.t[1], a.t[2]
        );
    }

    let mut materials: Vec<Shading> = Vec::new();
    let mut material_of_slot: HashMap<String, usize> = HashMap::new();
    // Textures are consumed as they are assigned to the first slot that uses them.
    let mut body_tex = body_tex;
    let mut part_a_tex = part_a_tex;
    let mut parts: Vec<Part> = Vec::new();
    let mut feet = f32::MAX;

    for (mesh, attach) in [(&body, None), (&head, attach)] {
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
                    let shading = shading_for(&slot, &mut body_tex, &mut part_a_tex);
                    materials.push(shading);
                    let i = materials.len() - 1;
                    material_of_slot.insert(slot.clone(), i);
                    i
                }
            };
            // Only the body (in component space) sets the floor; the attached head would skew it.
            let mut ignored_feet = f32::MAX;
            let feet_ref = if attach.is_none() {
                &mut feet
            } else {
                &mut ignored_feet
            };
            parts.push(Part {
                mesh: section_mesh(lod, section, attach, feet_ref)
                    .with_context(|| format!("building section for slot {slot}"))?,
                material,
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
        feet_offset,
    })
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
            assets
                .materials
                .iter()
                .any(|s| matches!(s, Shading::Texture(_))),
            "at least one decoded game texture must shade a section"
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
    fn slots_choose_their_shading() {
        let tex = || Texture {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        };
        let skin = shading_for("Skin", &mut Some(tex()), &mut Some(tex()));
        assert!(matches!(skin, Shading::Texture(_)));
        let part = shading_for("PartA", &mut Some(tex()), &mut Some(tex()));
        assert!(matches!(part, Shading::Texture(_)));
        let glass = shading_for("GlassA", &mut Some(tex()), &mut Some(tex()));
        assert!(matches!(glass, Shading::Solid(_)));
    }
}
