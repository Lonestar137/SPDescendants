//! Print the sound aliases an MW2 install's zones hold, and which of them carry audio.
//!
//! A development instrument for choosing the one alias per game event the app plays, and for
//! seeing how far each zone's walk gets:
//!
//! ```sh
//! cargo run -p spdesc-mw2 --example sounds              # the resolved MW2_DATA, the effect zones
//! cargo run -p spdesc-mw2 --example sounds -- /path/to/mw2 common
//! cargo run -p spdesc-mw2 --example sounds -- /path/to/mw2 common --match footstep
//! ```

use std::path::PathBuf;

use spdesc_mw2::{SoundBank, ZONES};

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let mut install: Option<PathBuf> = None;
    let mut zones: Vec<String> = Vec::new();
    let mut patterns: Vec<String> = Vec::new();
    let mut dump: Option<PathBuf> = None;
    let mut tables = false;
    let mut show: Vec<String> = Vec::new();
    let mut images: Option<PathBuf> = None;
    let mut raw: Vec<String> = Vec::new();
    let mut list: Vec<String> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--match" => {
                if let Some(p) = args.next() {
                    patterns.push(p.to_lowercase());
                }
            }
            "--zones" => zones.extend(args.by_ref()),
            "--dump" => dump = args.next().map(PathBuf::from),
            "--tables" => tables = true,
            "--list" => {
                if let Some(name) = args.next() {
                    list.push(name);
                }
            }
            "--raw" => {
                if let Some(name) = args.next() {
                    raw.push(name);
                }
            }
            "--images" => images = args.next().map(PathBuf::from),
            "--show" => {
                if let Some(name) = args.next() {
                    show.push(name);
                }
            }
            "--help" | "-h" => {
                println!("usage: sounds [install] [zone...] [--match text]... [--dump dir]");
                return std::process::ExitCode::SUCCESS;
            }
            other => {
                if install.is_none() && PathBuf::from(other).is_dir() {
                    install = Some(PathBuf::from(other));
                } else {
                    zones.push(other.to_owned());
                }
            }
        }
    }
    let Some(install) = install.or_else(spdesc_game_core::mw2_data) else {
        eprintln!("no MW2_DATA folder found; pass an install root as the first argument");
        return std::process::ExitCode::FAILURE;
    };
    let zone_list: Vec<&str> = if zones.is_empty() {
        ZONES.to_vec()
    } else {
        zones.iter().map(String::as_str).collect()
    };

    println!("install: {}", install.display());
    let bank = SoundBank::read(&install, &zone_list);
    for (zone, failure) in bank.failures() {
        println!("{zone}: FAILED — {failure}");
    }
    for (zone, set) in bank.zones() {
        println!(
            "{zone}: {} aliases, {} loaded payloads, {} assets walked{}",
            set.aliases.len(),
            set.loaded.len(),
            set.walked,
            match &set.stopped_at {
                Some(why) => format!(", stopped: {why}"),
                None => String::new(),
            }
        );
    }
    println!("total aliases: {}", bank.alias_count());
    if let Some(dir) = &images
        && let Err(error) = dump_icons(&install, dir)
    {
        eprintln!("cannot write the icons: {error}");
        return std::process::ExitCode::FAILURE;
    }
    for wanted in &show {
        for (zone, data) in bank.zones() {
            if let Some(table) = data.table(wanted) {
                println!(
                    "\n{zone}: {} ({}x{})",
                    table.name, table.columns, table.rows
                );
                for row in 0..table.rows {
                    let cells: Vec<&str> = (0..table.columns)
                        .map(|c| table.cell(row, c).unwrap_or(""))
                        .collect();
                    println!("  {row:>3}: {}", cells.join(" | "));
                }
            }
        }
    }
    for needle in &list {
        let names = spdesc_mw2::iwd::entry_names(&install, needle);
        println!("\n{} archive entry name(s) for `{needle}`:", names.len());
        for name in names.iter().take(200) {
            println!("  {name}");
        }
    }
    for wanted in &raw {
        for (zone, data) in bank.zones() {
            let Some(file) = data.raw_file(wanted) else {
                continue;
            };
            println!(
                "\n{zone}: {} ({} bytes{})",
                file.name,
                file.bytes.len(),
                if file.zlib_compressed { ", zlib" } else { "" }
            );
            match file.data() {
                Ok(bytes) => println!("{}", String::from_utf8_lossy(&bytes)),
                Err(error) => println!("cannot read it: {error}"),
            }
        }
    }
    if tables {
        for (zone, data) in bank.zones() {
            println!(
                "\n{zone}: {} string table(s), {} raw file(s)",
                data.tables.len(),
                data.raw_files.len()
            );
            for table in &data.tables {
                println!("  table {} ({}x{})", table.name, table.columns, table.rows);
            }
            for file in &data.raw_files {
                println!(
                    "  raw   {} ({} bytes{})",
                    file.name,
                    file.bytes.len(),
                    if file.zlib_compressed { ", zlib" } else { "" }
                );
            }
        }
    }
    if let Some(dir) = &dump
        && let Err(error) = std::fs::create_dir_all(dir)
    {
        eprintln!("cannot create {}: {error}", dir.display());
        return std::process::ExitCode::FAILURE;
    }

    let names = if patterns.is_empty() {
        bank.alias_names()
    } else {
        bank.alias_names()
            .into_iter()
            .filter(|name| {
                let lower = name.to_lowercase();
                patterns.iter().any(|p| lower.contains(p.as_str()))
            })
            .collect()
    };
    println!("\n{} name(s):", names.len());
    for name in names.iter().take(4000) {
        match bank.find(name) {
            Some((alias, Some(payload))) => println!(
                "  {name}  [{} bytes, {} s, fmt {}, {} Hz x{}, {} bit{}]",
                payload.bytes.len(),
                payload
                    .duration_seconds()
                    .map(|s| format!("{s:.3}"))
                    .unwrap_or_else(|| "?".into()),
                payload.format,
                payload.rate,
                payload.channels,
                payload.bits,
                if alias.looping { ", loop" } else { "" }
            ),
            Some((alias, None)) => match &alias.streamed {
                Some((dir, file)) => println!("  {name}  [streamed {dir}/{file}]"),
                None => println!("  {name}  [no payload]"),
            },
            None => println!("  {name}"),
        }
        if let (Some(dir), Some((_, Some(payload)))) = (&dump, bank.find(name))
            && let Some(wav) = payload.to_wav()
        {
            let file = dir.join(format!("{name}.wav"));
            if let Err(error) = std::fs::write(&file, wav) {
                eprintln!("cannot write {}: {error}", file.display());
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    if let Some(dir) = &dump {
        println!("\nwrote the payloads to {}", dir.display());
    }
    std::process::ExitCode::SUCCESS
}

/// Write the rank and prestige icons out as PPM, for looking at the decoded art.
fn dump_icons(install: &std::path::Path, dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let bank = SoundBank::read(install, &["code_post_gfx_mp"]);
    let zone = bank
        .zones()
        .first()
        .map(|(_, data)| data)
        .ok_or("code_post_gfx_mp did not open")?;
    let table = zone
        .table("mp/rankicontable.csv")
        .ok_or("no mp/rankicontable.csv")?;

    let mut entries: Vec<(String, String)> = Vec::new();
    for rank in 0..70usize {
        if let Some(name) = table.cell(rank + 2, 1) {
            let file = format!("rank-{rank:02}-{name}");
            entries.push((spdesc_mw2::iwd::rank_icon_entry(name), file));
        }
    }
    for prestige in 1..=10u32 {
        entries.push((
            spdesc_mw2::iwd::prestige_icon_entry(prestige),
            format!("prestige-{prestige:02}"),
        ));
    }

    let mut written = 0;
    for (entry, name) in entries {
        let Some(bytes) = spdesc_mw2::iwd::read_entry(install, &entry) else {
            eprintln!("missing {entry}");
            continue;
        };
        let image = match spdesc_mw2::iwi::decode(&bytes) {
            Ok(image) => image,
            Err(error) => {
                eprintln!("{entry}: {error}");
                continue;
            }
        };
        let mut ppm = format!("P6\n{} {}\n255\n", image.width, image.height).into_bytes();
        for texel in image.rgba.as_chunks::<4>().0 {
            // Composite over mid grey so an alpha shape is visible in a PPM.
            let a = u32::from(texel[3]);
            for channel in &texel[0..3] {
                ppm.push(((u32::from(*channel) * a + 110 * (255 - a)) / 255) as u8);
            }
        }
        let file = dir.join(format!("{name}.ppm"));
        std::fs::write(&file, ppm).map_err(|e| e.to_string())?;
        written += 1;
    }
    println!("wrote {written} icon(s) to {}", dir.display());
    Ok(())
}
