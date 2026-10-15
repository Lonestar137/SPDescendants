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

use spdesc_mw2::{EFFECT_ZONES, SoundBank};

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let mut install: Option<PathBuf> = None;
    let mut zones: Vec<String> = Vec::new();
    let mut patterns: Vec<String> = Vec::new();
    let mut dump: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--match" => {
                if let Some(p) = args.next() {
                    patterns.push(p.to_lowercase());
                }
            }
            "--zones" => zones.extend(args.by_ref()),
            "--dump" => dump = args.next().map(PathBuf::from),
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
        EFFECT_ZONES.to_vec()
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
