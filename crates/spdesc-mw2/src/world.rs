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
    AssetType, GfxWorldGeometry, Ptr, XFILE_BLOCK_VIRTUAL, ZoneStream, load_asset_at_with,
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
    }
}
