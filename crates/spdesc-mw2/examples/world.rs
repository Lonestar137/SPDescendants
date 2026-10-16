//! Measure a Modern Warfare 2 map zone: what it holds, how heavy its world is, and how its
//! surfaces divide the index buffer between the materials they draw with.
//!
//! A development instrument for deciding what rendering a map involves — how many surfaces,
//! vertices and indices its world has, how many materials it draws with, and how many textures
//! those materials name:
//!
//! ```sh
//! cargo run -p spdesc-mw2 --example world                 # a few shipped maps
//! cargo run -p spdesc-mw2 --example world -- /path/to/mw2 mp_rust mp_estate
//! ```

use std::path::PathBuf;

/// One Modern Warfare 2 world unit in metres: the engine's unit is an inch.
const MW2_METRES_PER_UNIT: f32 = 0.0254;

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let mut install: Option<PathBuf> = None;
    let mut maps: Vec<String> = Vec::new();
    for arg in args.by_ref() {
        if install.is_none() && PathBuf::from(&arg).is_dir() {
            install = Some(PathBuf::from(&arg));
        } else {
            maps.push(arg);
        }
    }
    let Some(install) = install.or_else(spdesc_game_core::mw2_data) else {
        eprintln!("no MW2_DATA folder found; pass an install root as the first argument");
        return std::process::ExitCode::FAILURE;
    };
    if maps.is_empty() {
        maps = ["mp_rust", "mp_highrise", "mp_estate", "mp_terminal"]
            .iter()
            .map(|m| (*m).to_owned())
            .collect();
    }

    for map in &maps {
        let path = spdesc_mw2::zone_path(&install, map);
        if !path.is_file() {
            println!("{map}: no zone at {}", path.display());
            continue;
        }
        let started = std::time::Instant::now();
        let image = match spdesc_mw2::open_zone(&path) {
            Ok(image) => image,
            Err(error) => {
                println!("{map}: cannot open the zone: {error}");
                continue;
            }
        };
        let report = match spdesc_mw2::read_map(&image, map) {
            Ok(report) => report,
            Err(error) => {
                println!("{map}: cannot walk the zone: {error}");
                continue;
            }
        };
        println!(
            "\n=== {map} ({} assets in {:.2} s) ===",
            report.assets,
            started.elapsed().as_secs_f32()
        );
        let mut types: Vec<(&String, &usize)> = report.by_type.iter().collect();
        types.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let shown: Vec<String> = types
            .iter()
            .take(8)
            .map(|(name, count)| format!("{name} {count}"))
            .collect();
        println!("  types: {}", shown.join(", "));
        match &report.world {
            Some(world) => {
                println!(
                    "  world: {} surfaces ({} static), {} vertices, {} indices = {} triangles",
                    world.surfaces,
                    world.static_surfaces,
                    world.vertices,
                    world.indices,
                    world.triangles
                );
                println!(
                    "  light: {} pages, {} used, sky image {}, lightmap image {}; {} skies",
                    world.lightmap_pages,
                    world.lightmaps,
                    world.has_sky_image,
                    world.has_lightmap_image,
                    world.skies
                );
            }
            None => println!("  world: none"),
        }
        let passes: usize = report.materials.iter().map(|m| m.passes).sum();
        let arguments: usize = report.materials.iter().map(|m| m.arguments).sum();
        println!(
            "  materials: {} ({} passes, {} arguments, {:.0} triangles each)",
            report.materials.len(),
            passes,
            arguments,
            report.triangles_per_material()
        );
        let mut named: Vec<&spdesc_mw2::MaterialCounts> = report.materials.iter().collect();
        named.sort_by_key(|m| std::cmp::Reverse(m.arguments));
        let shown: Vec<String> = named
            .iter()
            .take(5)
            .map(|m| format!("{} ({} args)", m.name, m.arguments))
            .collect();
        if !shown.is_empty() {
            println!("  heaviest: {}", shown.join(", "));
        }
        match spdesc_mw2::read_clip(&image) {
            Ok(clip) => {
                let (min, max) = clip.extent();
                let walkable = clip.walkable.iter().filter(|walk| **walk).count();
                println!(
                    "  collision: {} vertices, {} triangles ({walkable} walkable), \
                     spans {:.0}x{:.0}x{:.0} m",
                    clip.positions.len(),
                    clip.triangle_count(),
                    (max[0] - min[0]) * MW2_METRES_PER_UNIT,
                    (max[1] - min[1]) * MW2_METRES_PER_UNIT,
                    (max[2] - min[2]) * MW2_METRES_PER_UNIT
                );
            }
            Err(error) => println!("  collision: cannot read: {error}"),
        }
        match spdesc_mw2::read_runs(&image) {
            Ok(runs) => {
                let materials: std::collections::HashSet<u32> =
                    runs.iter().map(|run| run.material).collect();
                let triangles: usize = runs.iter().map(|run| run.count / 3).sum();
                let widest = runs.iter().map(|run| run.count / 3).max().unwrap_or(0);
                let narrowest = runs.iter().map(|run| run.count / 3).min().unwrap_or(0);
                println!(
                    "  runs: {} surfaces over {} materials, {triangles} triangles ({}..{} each)",
                    runs.len(),
                    materials.len(),
                    narrowest,
                    widest
                );
            }
            Err(error) => println!("  runs: cannot read: {error}"),
        }
    }
    std::process::ExitCode::SUCCESS
}
