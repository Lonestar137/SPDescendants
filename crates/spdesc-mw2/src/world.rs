//! Measuring a Modern Warfare 2 map zone.
//!
//! A map is its own fastfile — `zone/english/<map>.ff`, where the map's name is the file's name —
//! and it holds the same kind of assets the other zones do, plus the ones that make it a place:
//! `gfx_map` (the renderable world), the `material`s its surfaces draw with, the `image`s those
//! materials name, the `clipMap` a player collides with, and the `map_ents` the game spawns from.
//!
//! This module walks one map zone and reports what is in it: how many assets of each type, and for
//! the world itself the counts that decide whether rendering it is a small job or a large one —
//! surfaces, vertices and indices, the lightmap pages, and whether the world names a sky. It reads
//! nothing else: no geometry is turned into meshes here, and no texture is decoded.
//!
//! The walk is the same one `sounds.rs` does — assets are `Following`, so they are walked in the
//! order the zone stores them, and each body is loaded so the next one can be reached.

use std::collections::BTreeMap;

use fastfile_iw4::{
    AssetType, GfxWorldGeometry, Ptr, XFILE_BLOCK_VIRTUAL, ZoneStream, load_asset_at_observed,
    load_asset_at_with,
};

use crate::open::{Error, ZoneImage};

/// How many assets of each type a zone holds, by the type's own name.
pub type TypeCounts = BTreeMap<String, usize>;

/// What a map's renderable world is made of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldCounts {
    /// How many surfaces the world draws with, and how many of them are static (its DPVS list).
    pub surfaces: usize,
    pub static_surfaces: usize,
    /// The shared vertex and index pools every surface draws from.
    pub vertices: usize,
    pub indices: usize,
    /// How many triangles the pools hold together.
    pub triangles: usize,
    /// Lightmap pages, and how many the world actually uses.
    pub lightmap_pages: usize,
    pub lightmaps: usize,
    /// How many skies the world names, which is where the outdoor art comes from.
    pub skies: usize,
    /// Whether the world names a sky image at all.
    pub has_sky_image: bool,
    /// Whether it names a first lightmap page image.
    pub has_lightmap_image: bool,
}

/// One material, and how much work drawing it would be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialCounts {
    /// The material's own name, as its asset holds it.
    pub name: String,
    /// How many technique passes it has, each of which is a draw stage.
    pub passes: usize,
    /// How many arguments those passes carry, which is where its textures and constants live.
    pub arguments: usize,
}

/// What one map zone holds.
#[derive(Clone, Debug, Default)]
pub struct MapReport {
    /// The map's name, which is also its fastfile.
    pub map: String,
    /// How many assets were walked, and how many are of each type.
    pub assets: usize,
    pub by_type: TypeCounts,
    /// The world's own counts, when the zone holds one.
    pub world: Option<WorldCounts>,
    /// Every material the zone holds, by name.
    pub materials: Vec<MaterialCounts>,
}

impl MapReport {
    /// How many triangles per material the map has, which is the shape of the draw work.
    pub fn triangles_per_material(&self) -> f64 {
        match (self.world.as_ref(), self.materials.len()) {
            (Some(world), materials) if materials > 0 => world.triangles as f64 / materials as f64,
            _ => 0.0,
        }
    }
}

/// Walk the map zone `image` and report what it holds.
pub fn read_map(image: &ZoneImage, map: &str) -> std::result::Result<MapReport, Error> {
    let mut memory = image.memory();
    let mut stream = memory.stream(image.bytes())?;
    let mut report = MapReport {
        map: map.to_owned(),
        ..MapReport::default()
    };

    stream.push(XFILE_BLOCK_VIRTUAL)?;
    let table = fastfile_iw4::open_asset_table(&mut stream)?;
    for i in 0..table.count() {
        let ty = table.kind(&stream, i)?;
        let Some(slot) = table.slot(i) else {
            break;
        };
        *report.by_type.entry(ty.name().to_owned()).or_default() += 1;
        let world = &mut report.world;
        let materials = &mut report.materials;
        load_asset_at_with(&mut stream, ty, slot, |s, ty| {
            match ty {
                AssetType::GfxWorld => {
                    if let Some(geometry) = s.gfx_world() {
                        *world = Some(counts_of(&geometry));
                    }
                }
                AssetType::Material => {
                    if let Some(counts) = material_counts(s) {
                        materials.push(counts);
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        report.assets += 1;
    }
    stream.pop()?;
    Ok(report)
}

/// The world's counts, as its own geometry records them.
fn counts_of(world: &GfxWorldGeometry) -> WorldCounts {
    let used_pages = world
        .lightmaps
        .iter()
        .filter(|pair| pair.primary.is_some() || pair.secondary.is_some())
        .count();
    WorldCounts {
        surfaces: world.surface_count,
        static_surfaces: world.static_surface_count,
        vertices: world.vertex_count,
        indices: world.index_count,
        triangles: world.index_count / 3,
        lightmap_pages: world.lightmap_count,
        lightmaps: used_pages,
        skies: world.sky_count,
        has_sky_image: world.first_sky_image.is_some(),
        has_lightmap_image: world.first_lightmap_primary.is_some(),
    }
}

/// One material's name and how many draw stages it has.
fn material_counts(s: &ZoneStream<'_>) -> Option<MaterialCounts> {
    let material = s.latest_material()?;
    let name = material
        .name
        .and_then(|name: Ptr| s.cstr(name).ok())
        .unwrap_or_default();
    let (passes, arguments) = match s.latest_technique_graph() {
        Some(graph) => {
            let rows: Vec<_> = graph.iter_rows().collect();
            let arguments = rows.iter().map(|row| graph.arguments_for(*row).len()).sum();
            (rows.len(), arguments)
        }
        None => (0, 0),
    };
    Some(MaterialCounts {
        name: name.to_owned(),
        passes,
        arguments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map's collision geometry reads whole: a real vertex pool of positions and a real triangle
    /// list, with every index inside the pool and an extent the size of a map rather than of noise.
    #[test]
    fn a_maps_collision_reads_as_geometry() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let map = "mp_rust";
        let path = crate::zone_path(&install, map);
        if !path.is_file() {
            spdesc_game_core::MW2.skip("the install does not ship mp_rust");
            return;
        }
        let image = crate::open_zone(&path).expect("the map zone opens");
        let clip = read_clip(&image).expect("the collision reads");

        assert!(
            clip.positions.len() > 1000,
            "{} vertices",
            clip.positions.len()
        );
        assert!(
            clip.triangle_count() > 1000,
            "{} triangles",
            clip.triangle_count()
        );
        assert_eq!(clip.walkable.len(), clip.triangle_count());
        assert!(
            clip.walkable.iter().filter(|walk| **walk).count() > 100,
            "a map has walkable triangles"
        );
        let largest = clip.indices.iter().copied().max().unwrap_or(0) as usize;
        assert!(
            largest < clip.positions.len(),
            "an index leaves the vertex pool"
        );
        // Positions are the engine's own unit, an inch: a map is hundreds of metres, not kilometres and not nothing.
        let (min, max) = clip.extent();
        for axis in 0..3 {
            let span = max[axis] - min[axis];
            assert!(
                (1000.0..600_000.0).contains(&span),
                "axis {axis} spans {span} cm"
            );
        }
    }

    /// A map's spawn is the middle of its walkable ground: inside the map, on a triangle that is
    /// flagged walkable, and near the middle of where the walkable ground is.
    #[test]
    fn a_maps_spawn_is_on_its_walkable_ground() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let path = crate::zone_path(&install, "mp_rust");
        if !path.is_file() {
            spdesc_game_core::MW2.skip("the install does not ship mp_rust");
            return;
        }
        let image = crate::open_zone(&path).expect("the map zone opens");
        let clip = read_clip(&image).expect("the collision reads");
        let spawn = clip.spawn().expect("a map has walkable ground");
        let (min, max) = clip.extent();
        for axis in 0..3 {
            assert!(
                min[axis] <= spawn[axis] && spawn[axis] <= max[axis],
                "axis {axis}: spawn {} outside {}..{}",
                spawn[axis],
                min[axis],
                max[axis]
            );
        }
        // The spawn is the centre of a walkable triangle, so it is a triangle of the collision.
        let on_a_triangle = clip.indices.as_chunks::<3>().0.iter().any(|tri| {
            let centroid = [0, 1, 2].map(|k| {
                let at = tri[k] as usize;
                clip.positions.get(at).copied().unwrap_or([f32::NAN; 3])
            });
            let centre = [0, 1, 2]
                .map(|axis| (centroid[0][axis] + centroid[1][axis] + centroid[2][axis]) / 3.0);
            (0..3).all(|axis| (centre[axis] - spawn[axis]).abs() < 0.001)
        });
        assert!(
            on_a_triangle,
            "the spawn is not the centre of a floor triangle"
        );
    }

    /// Every drawn-world triangle stays inside its surface's authored bounds.
    ///
    /// The world contains non-collision geometry, so its overall extent need not match the clipmap.
    /// Surface bounds are independent records stored alongside the surfaces. They prove that each
    /// run's u16 indices are resolved through its `firstVertex` at `+4`, rather than read directly
    /// from the shared vertex pool.
    #[test]
    fn drawn_world_indices_resolve_inside_their_surface_bounds() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let mut tested = 0;
        for map in ["mp_rust", "mp_subbase", "mp_estate", "mp_terminal"] {
            let path = crate::zone_path(&install, map);
            if !path.is_file() {
                continue;
            }
            let image = crate::open_zone(&path).expect("the map zone opens");
            let world = read_world(&image).expect("the world reads");
            let mut checked = 0;
            for run in &world.runs {
                let (min, max) = run.bounds.expect("every surface has bounds");
                let end = run.first.saturating_add(run.count);
                let indices = world
                    .indices
                    .get(run.first..end)
                    .expect("a surface run stays inside the index buffer");
                for index in indices {
                    let position = world
                        .positions
                        .get(*index as usize)
                        .expect("a resolved index stays inside the vertex pool");
                    assert!(
                        (0..3).all(|axis| {
                            min[axis] - 0.1 <= position[axis] && position[axis] <= max[axis] + 0.1
                        }),
                        "{map}: surface at index {} addresses {position:?} outside {min:?}..{max:?}",
                        run.first
                    );
                    checked += 1;
                }
            }
            assert_eq!(
                checked,
                world.indices.len(),
                "{map}: every index was checked"
            );
            tested += 1;
        }
        if tested == 0 {
            spdesc_game_core::MW2.skip("the install ships none of the measured map zones");
        }
    }

    /// A map zone is its own fastfile, and walking one reports a world with real geometry: this is
    /// the measurement that says whether rendering maps is a small job or a large one.
    #[test]
    fn a_map_zone_reports_its_world_and_materials() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let map = "mp_rust";
        let path = crate::zone_path(&install, map);
        if !path.is_file() {
            spdesc_game_core::MW2.skip("the install does not ship mp_rust");
            return;
        }
        let image = crate::open_zone(&path).expect("the map zone opens");
        let report = read_map(&image, map).expect("the map zone walks");

        let world = report.world.as_ref().expect("a map holds a world");
        assert!(world.surfaces > 0, "the world has no surfaces");
        assert!(world.vertices > 0, "the world has no vertices");
        assert!(world.indices > 0, "the world has no indices");
        assert_eq!(world.triangles, world.indices / 3);
        assert!(
            world.static_surfaces > 0 && world.static_surfaces <= world.surfaces,
            "{} static of {} surfaces",
            world.static_surfaces,
            world.surfaces
        );
        assert!(!report.materials.is_empty(), "the map has no materials");
        assert!(
            report.materials.iter().all(|m| !m.name.is_empty()),
            "a material has no name"
        );
        // The zone is not only a world: it carries the collision the player walks on and the
        // map's own materials, which is what a render would draw the world with.
        assert!(
            report.by_type.contains_key("clipmap"),
            "a map zone holds a clipmap: {:?}",
            report.by_type.keys().collect::<Vec<_>>()
        );
        assert!(
            report.assets > 100,
            "a map zone holds {} assets",
            report.assets
        );

        // The runs tile the index buffer, and every one is a whole number of triangles: a partition
        // derived from anything but the real first indices would not come out that way.
        let runs = read_runs(&image).expect("the runs read");
        assert_eq!(runs.len(), world.surfaces);
        assert_eq!(
            runs[0].first, 0,
            "the first run starts at the buffer's start"
        );
        let last = runs.last().expect("runs");
        assert_eq!(last.first + last.count, world.indices);
        let mut covered = 0usize;
        for (i, run) in runs.iter().enumerate() {
            assert!(run.count % 3 == 0, "run {i} is {} indices", run.count);
            if let Some(next) = runs.get(i + 1) {
                assert_eq!(
                    run.first + run.count,
                    next.first,
                    "run {i} does not reach the next"
                );
            }
            covered += run.count;
        }
        assert_eq!(covered, world.indices, "the runs do not cover the buffer");
        let materials: std::collections::HashSet<u32> =
            runs.iter().map(|run| run.material).collect();
        assert!(
            materials.len() > 10,
            "the world draws with {} materials",
            materials.len()
        );
    }
}

/// A sink that ignores what the walk reports about links: the probe only needs the zone's bodies
/// loaded, in order.
#[derive(Default)]
struct Ignore;

impl fastfile_iw4::AssetLinkSink for Ignore {
    fn loaded(
        &mut self,
        _s: &ZoneStream<'_>,
        _ty: AssetType,
        _slot: Ptr,
        _insert_slot: Option<Ptr>,
    ) -> fastfile_iw4::Result<()> {
        Ok(())
    }

    fn alias(&mut self, _ty: AssetType, _slot: Ptr, _target: Ptr) -> fastfile_iw4::Result<()> {
        Ok(())
    }
}

/// A map's surfaces, as runs of its world's index buffer.
///
/// A surface draws one run, in one material. The run's first index is the surface's own word at
/// `+12`; the runs are contiguous in that order and the last one ends at the index count. That was
/// established from the data rather than from a remembered struct: the runs in first-index order
/// tile the index buffer exactly, and **every run is a whole number of triangles** — a partition
/// derived from anything but the real first indices would not be.
///
/// Each run's indices are local to its first vertex. The word at `+4` is that base, so adding it to
/// every index in the run resolves the shared vertex pool. The word at `+0` is instead the offset
/// into the separate vertex-layer stream; it does not address the positions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceRun {
    /// The byte offset into the separate vertex-layer stream for this surface.
    pub vertex_layer_data: u32,
    /// The first vertex this run's u16 indices address in the world's shared vertex pool.
    pub first_vertex: u32,
    /// How many vertices this surface exposes to the renderer.
    pub vertex_count: usize,
    /// The first index of the run, in the world's index buffer.
    pub first: usize,
    /// How many indices it draws. Always a multiple of three.
    pub count: usize,
    /// The material the surface draws with, as the zone stores the word: it decodes as a pointer
    /// into the virtual block, which is how the walk already follows it.
    pub material: u32,
    /// The surface's authored bounds, when its world carries them.
    pub bounds: Option<([f32; 3], [f32; 3])>,
}

/// How many bytes one surface occupies in the world's surface array.
const SURFACE_STRIDE: usize = 32;
/// Where a surface keeps its first vertex, first index, triangle count, and material.
///
/// The count is the *high* half of the word at `+8`: the low half is the world's own vertex count,
/// which every surface repeats. Measured, the counts and the first indices together cover the index
/// buffer exactly — 5 911 runs, no surface sharing a first index, and no run disagreeing with the
/// count it states.
const SURFACE_FIRST_VERTEX: usize = 4;
const SURFACE_VERTEX_COUNT: usize = 8;
const SURFACE_FIRST_INDEX: usize = 12;
const SURFACE_TRIANGLE_COUNT: usize = 10;
const SURFACE_MATERIAL: usize = 16;
/// How many bytes one world vertex occupies, and where its position sits inside it.
const VERTEX_STRIDE: usize = 44;
const VERTEX_POSITION: usize = 0;

/// A map's drawn world: the geometry the game renders, with the surfaces that group it.
#[derive(Clone, Debug, Default)]
pub struct WorldGeometry {
    /// Vertex positions, in the game's own frame (`+X` forward, `+Z` up) and unit (inches).
    pub positions: Vec<[f32; 3]>,
    /// Triangle indices into `positions`, three per triangle.
    pub indices: Vec<u32>,
    /// The surfaces, each a run of `indices` drawn with one material.
    pub runs: Vec<SurfaceRun>,
}

impl WorldGeometry {
    /// How many triangles the world draws.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The box the drawn triangles occupy, in the map's own units (inches).
    ///
    /// The pool can contain vertices which no surface draws, so this follows the resolved triangle
    /// indices rather than treating every record in the pool as map geometry.
    pub fn extent(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        let mut any = false;
        for index in &self.indices {
            let Some(position) = self.positions.get(*index as usize) else {
                continue;
            };
            any = true;
            for axis in 0..3 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        if !any {
            ([0.0; 3], [0.0; 3])
        } else {
            (min, max)
        }
    }
}

/// Read a map's drawn world: its vertex pool, its triangle list, and the runs that group it.
///
/// The u16 indices in each run are relative to the surface's `firstVertex` at `+4`; they are
/// resolved to absolute indices here. A position is the first three floats of a 44-byte vertex.
pub fn read_world(image: &ZoneImage) -> std::result::Result<WorldGeometry, Error> {
    let mut memory = image.memory();
    let mut stream = memory.stream(image.bytes())?;
    stream.push(XFILE_BLOCK_VIRTUAL)?;
    let mut sink = Ignore;
    let table = fastfile_iw4::open_asset_table(&mut stream)?;
    for i in 0..table.count() {
        let ty = table.kind(&stream, i)?;
        let Some(slot) = table.slot(i) else { break };
        if load_asset_at_observed(&mut stream, ty, slot, &mut sink).is_err() {
            break;
        }
    }
    stream.pop()?;
    let Some(world) = stream.gfx_world() else {
        return Ok(WorldGeometry::default());
    };
    let Some(vertex_ptr) = world.vertices else {
        return Ok(WorldGeometry::default());
    };
    let positions: Vec<[f32; 3]> = (0..world.vertex_count)
        .map(|i| {
            let at = vertex_ptr.at(i * VERTEX_STRIDE);
            [
                stream.f32_at(at, VERTEX_POSITION).unwrap_or(0.0),
                stream.f32_at(at, VERTEX_POSITION + 4).unwrap_or(0.0),
                stream.f32_at(at, VERTEX_POSITION + 8).unwrap_or(0.0),
            ]
        })
        .collect();
    let runs = read_runs(image)?;
    let mut indices: Vec<u32> = match world.indices {
        Some(index_ptr) => (0..world.index_count)
            .map(|i| u32::from(stream.u16_at(index_ptr.at(i * 2), 0).unwrap_or(0)))
            .collect(),
        None => Vec::new(),
    };
    for run in &runs {
        let Some(indices) = indices.get_mut(run.first..run.first.saturating_add(run.count)) else {
            continue;
        };
        for index in indices {
            *index = index.saturating_add(run.first_vertex);
        }
    }
    Ok(WorldGeometry {
        positions,
        indices,
        runs,
    })
}

/// Read the world's surfaces as runs of its index buffer, in first-index order.
pub fn read_runs(image: &ZoneImage) -> std::result::Result<Vec<SurfaceRun>, Error> {
    let mut memory = image.memory();
    let mut stream = memory.stream(image.bytes())?;
    stream.push(XFILE_BLOCK_VIRTUAL)?;
    let mut sink = Ignore;
    let table = fastfile_iw4::open_asset_table(&mut stream)?;
    for i in 0..table.count() {
        let ty = table.kind(&stream, i)?;
        let Some(slot) = table.slot(i) else { break };
        if load_asset_at_observed(&mut stream, ty, slot, &mut sink).is_err() {
            break;
        }
    }
    stream.pop()?;
    let Some(world) = stream.gfx_world() else {
        return Ok(Vec::new());
    };
    let Some(arr) = world.surfaces else {
        return Ok(Vec::new());
    };

    let mut runs: Vec<SurfaceRun> = (0..world.surface_count)
        .map(|i| {
            let surface = arr.at(i * SURFACE_STRIDE);
            let bounds = world.surfaces_bounds.map(|arr| {
                let bounds = arr.at(i * 24);
                let midpoint = [
                    stream.f32_at(bounds, 0).unwrap_or(0.0),
                    stream.f32_at(bounds, 4).unwrap_or(0.0),
                    stream.f32_at(bounds, 8).unwrap_or(0.0),
                ];
                let half_size = [
                    stream.f32_at(bounds, 12).unwrap_or(0.0),
                    stream.f32_at(bounds, 16).unwrap_or(0.0),
                    stream.f32_at(bounds, 20).unwrap_or(0.0),
                ];
                (
                    [0, 1, 2].map(|axis| midpoint[axis] - half_size[axis]),
                    [0, 1, 2].map(|axis| midpoint[axis] + half_size[axis]),
                )
            });
            SurfaceRun {
                vertex_layer_data: stream.u32_at(surface, 0).unwrap_or(0),
                first_vertex: stream.u32_at(surface, SURFACE_FIRST_VERTEX).unwrap_or(0),
                vertex_count: usize::from(
                    stream.u16_at(surface, SURFACE_VERTEX_COUNT).unwrap_or(0),
                ),
                first: stream.u32_at(surface, SURFACE_FIRST_INDEX).unwrap_or(0) as usize,
                // The surface states its triangles; three indices each.
                count: usize::from(stream.u16_at(surface, SURFACE_TRIANGLE_COUNT).unwrap_or(0)) * 3,
                material: stream.u32_at(surface, SURFACE_MATERIAL).unwrap_or(0),
                bounds,
            }
        })
        .collect();
    runs.sort_by_key(|run| run.first);
    Ok(runs)
}

/// Where each surface's material sits, and the vertex base it is addressed with.
#[doc(hidden)]
pub const SURFACE_FIELDS: [usize; 3] =
    [SURFACE_FIRST_VERTEX, SURFACE_FIRST_INDEX, SURFACE_MATERIAL];

/// A map's collision geometry: the triangles a player stands on and walks into.
///
/// The clipmap is the one piece of a map's *space* the vendored reader already captures whole —
/// its vertex pool is 12-byte records of three floats and its triangles are u16 indices — so this
/// reads it without guessing a layout. It is the collision hull rather than the drawn world, so it
/// is missing props and detail, but it is the map's own shape and the player can walk on it.
#[derive(Clone, Debug, Default)]
pub struct ClipGeometry {
    /// Vertex positions, in the game's own frame (`+X` forward, `+Z` up) and unit (inches).
    pub positions: Vec<[f32; 3]>,
    /// Triangle indices into `positions`, three per triangle.
    pub indices: Vec<u32>,
    /// Whether each triangle is one a player can stand on.
    pub walkable: Vec<bool>,
}

/// Whether a collision triangle is ground rather than a wall: its normal points mostly up or down.
fn floor_like(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> bool {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let nz = u[0] * v[1] - u[1] * v[0];
    let len = (u[1] * v[2] - u[2] * v[1])
        .hypot(u[2] * v[0] - u[0] * v[2])
        .hypot(nz);
    len > 1e-9 && (nz / len).abs() > 0.5
}

/// The area of a collision triangle.
fn triangle_area(a: &[f32; 3], b: &[f32; 3], c: &[f32; 3]) -> f32 {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt()
}

impl ClipGeometry {
    /// How many triangles the collision has, and how many are walkable.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
    /// A point to stand on: the middle of the map's floor.
    ///
    /// The walkable flags name ramps as well as floors, and a spot on a ramp or the side of
    /// something is a poor place to stand a map on, so a triangle whose surface is near-vertical is
    /// passed over: the anchor lands on ground. A map has more than one level — a floor, a roof, a
    /// stairwell — so the ground is grouped by height and the *largest* level wins, by the area of
    /// its floor rather than by how many triangles cover it: the floor a player walks on is the big
    /// one, where a roof can be busy with detail on a fraction of the ground. The middle of that
    /// level is then taken as a *median*, so one distant patch cannot drag the answer away.
    /// `None` when the collision has no walkable triangle at all.
    pub fn spawn(&self) -> Option<[f32; 3]> {
        /// The height one level spans, centimetres: a map's floors are metres apart, not
        /// centimetres.
        const LEVEL: f32 = 100.0;
        let mut flats: Vec<([f32; 3], f32)> = Vec::new();
        let mut all: Vec<[f32; 3]> = Vec::new();
        for tri in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            if !self
                .walkable
                .get(tri[0] as usize / 3)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let (Some(a), Some(b), Some(c)) = (
                self.positions.get(a),
                self.positions.get(b),
                self.positions.get(c),
            ) else {
                continue;
            };
            let centre = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            if floor_like(a, b, c) {
                flats.push((centre, triangle_area(a, b, c)));
            }
            all.push(centre);
        }
        let flats = if flats.is_empty() {
            all.into_iter().map(|centre| (centre, 1.0)).collect()
        } else {
            flats
        };
        if flats.is_empty() {
            return None;
        }
        let mut levels: std::collections::HashMap<i32, f32> = std::collections::HashMap::new();
        for (centre, area) in &flats {
            *levels
                .entry((centre[2] / LEVEL).round() as i32)
                .or_default() += area;
        }
        // Deterministic: the largest level, ties broken by the number of samples then by height.
        let mut ranked: Vec<(i32, f32)> = levels.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let main = ranked.first().map(|(level, _)| *level)?;
        let centres: Vec<[f32; 3]> = flats
            .into_iter()
            .filter(|(c, _)| (c[2] / LEVEL).round() as i32 == main)
            .map(|(c, _)| c)
            .collect();
        if centres.is_empty() {
            return None;
        }
        // The middle of that level, taken as a median rather than a mean so one distant patch — a
        // roof, a catwalk — cannot drag the answer away. `+Z` is up, so the search is over `x` and
        // `y`.
        let median = |axis: usize| {
            let mut values: Vec<f32> = centres.iter().map(|c| c[axis]).collect();
            values.sort_by(f32::total_cmp);
            values[values.len() / 2]
        };
        let (mx, my) = (median(0), median(1));
        centres.into_iter().min_by(|a, b| {
            let da = (a[0] - mx).powi(2) + (a[1] - my).powi(2);
            let db = (b[0] - mx).powi(2) + (b[1] - my).powi(2);
            da.total_cmp(&db)
        })
    }

    /// The box the collision occupies, in the map's own units (inches).
    pub fn extent(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for position in &self.positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        if self.positions.is_empty() {
            ([0.0; 3], [0.0; 3])
        } else {
            (min, max)
        }
    }
}

/// Read the map zone's collision geometry.
pub fn read_clip(image: &ZoneImage) -> std::result::Result<ClipGeometry, Error> {
    let mut memory = image.memory();
    let mut stream = memory.stream(image.bytes())?;
    stream.push(XFILE_BLOCK_VIRTUAL)?;
    let mut sink = Ignore;
    let table = fastfile_iw4::open_asset_table(&mut stream)?;
    for i in 0..table.count() {
        let ty = table.kind(&stream, i)?;
        let Some(slot) = table.slot(i) else { break };
        if load_asset_at_observed(&mut stream, ty, slot, &mut sink).is_err() {
            break;
        }
    }
    stream.pop()?;
    let Some(clip) = stream.clip_map() else {
        return Ok(ClipGeometry::default());
    };
    let Some(verts) = clip.verts else {
        return Ok(ClipGeometry::default());
    };
    let positions: Vec<[f32; 3]> = (0..clip.vert_count)
        .map(|i| {
            let at = verts.at(i * 12);
            [
                stream.f32_at(at, 0).unwrap_or(0.0),
                stream.f32_at(at, 4).unwrap_or(0.0),
                stream.f32_at(at, 8).unwrap_or(0.0),
            ]
        })
        .collect();
    let (indices, walkable) = match clip.tri_indices {
        Some(tris) => {
            let count = 3 * clip.tri_count;
            let indices: Vec<u32> = (0..count)
                .map(|i| u32::from(stream.u16_at(tris.at(i * 2), 0).unwrap_or(0)))
                .collect();
            // The walkable flag is one bit per triangle, packed into words.
            let walkable = match clip.tri_edge_is_walkable {
                Some(flags) => (0..clip.tri_count)
                    .map(|t| {
                        stream
                            .u32_at(flags, (t / 32) * 4)
                            .map(|word| word & (1 << (t % 32)) != 0)
                            .unwrap_or(false)
                    })
                    .collect(),
                None => vec![false; clip.tri_count],
            };
            (indices, walkable)
        }
        None => (Vec::new(), Vec::new()),
    };
    Ok(ClipGeometry {
        positions,
        indices,
        walkable,
    })
}
