//! Reading the player's read-only `TFD_DATA` folder: the IoStore container and the Zen package
//! summaries inside it.
//!
//! This crate owns the *what the files say* layer — container tables, decompression, package
//! structure — and nothing about the simulation or about presentation. It reads the folder
//! resolved by [`spdesc_game_core::tfd_data`] and never writes into it; nothing it returns is
//! committed (`docs/METHOD.md`).
//!
//! Three stages, each independently testable:
//! 1. [`key`] — resolve the container AES key from the local-state folder (`spdesc-config/`),
//!    never from the repository.
//! 2. [`container::IoStore`] — the `.utoc` tables and `.ucas` blocks: chunk id → decompressed
//!    bytes (per-block AES-256-ECB, then Oodle when the block is compressed).
//! 3. [`zen::ZenPackage`] — a package chunk's summary, name maps, and export map.
//!
//! Format references: CUE4Parse `FIoStoreTocResource`, `FZenPackageSummary`, `FExportMapEntry`,
//! `FMappedName`, and `FNameEntrySerialized`.

pub mod anim;
pub mod container;
pub mod key;
pub mod mapping;
pub mod mesh;
pub mod property;
pub mod skeletal;
pub mod texture;
pub mod zen;

use std::path::PathBuf;

/// The `*.utoc`/`*.ucas` pair beside each other under `<TFD_DATA>/M1/Content/Paks`, when the
/// folder resolves.
pub fn paks_dir() -> Option<PathBuf> {
    let dir = spdesc_game_core::tfd_data()?.join("M1/Content/Paks");
    dir.join("M1-Windows.utoc").is_file().then_some(dir)
}
