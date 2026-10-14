//! Zen package structure: the summary, name maps, and export map of one cooked package chunk.
//!
//! A Zen (UE5 IoStore) package begins with `FZenPackageSummary`, followed by the package-local name
//! batch, the import map, the export map (`FExportMapEntry`, 72 B each), and the export bundle
//! entries. Cooked export bytes follow the header. Names are `FMappedName` (a 30-bit index plus a
//! type in the top two bits): type 0 selects the package-local map, any other type the global map.
//!
//! `global.ucas` supplies the global name batch and the script-object table used to resolve
//! imported class paths.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn i32le(b: &[u8], at: usize) -> i32 {
    u32le(b, at) as i32
}
fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// `FNameEntrySerialized.LoadNameBatch`: count, string-bytes, hash version, per-entry hashes, u16
/// length headers (bit 0x80 selects UTF-16), then the packed strings. Returns the names and the
/// offset just past the batch.
fn parse_name_batch(b: &[u8], at: usize) -> Result<(Vec<String>, usize)> {
    if at + 16 > b.len() {
        bail!("name batch header overruns");
    }
    let count = u32le(b, at) as usize;
    let string_bytes = u32le(b, at + 4) as usize;
    let headers = at + 16 + count * 8;
    let strings_at = headers + count * 2;
    if count > 5_000_000 || strings_at + string_bytes > b.len() {
        bail!("name batch out of range");
    }
    let mut names = Vec::with_capacity(count);
    let mut p = strings_at;
    for i in 0..count {
        let h0 = b[headers + i * 2];
        let h1 = b[headers + i * 2 + 1];
        let nchars = (((h0 & 0x7f) as usize) << 8) | h1 as usize;
        if h0 & 0x80 != 0 {
            let s = String::from_utf16_lossy(
                &(0..nchars).map(|k| u16le(b, p + k * 2)).collect::<Vec<_>>(),
            );
            p += nchars * 2;
            names.push(s);
        } else {
            names.push(String::from_utf8_lossy(&b[p..p + nchars]).into_owned());
            p += nchars;
        }
    }
    Ok((names, strings_at + string_bytes))
}

/// `FMappedName`: `u32 _nameIndex` (30-bit index, top 2 bits type) + `u32 ExtraIndex`.
fn mapped_name(b: &[u8], at: usize, local: &[String], global: &[String]) -> String {
    let raw = u32le(b, at);
    let extra = u32le(b, at + 4);
    let idx = (raw & 0x3FFF_FFFF) as usize;
    let table: &[String] = if raw >> 30 != 0 { global } else { local };
    let base = table
        .get(idx)
        .cloned()
        .unwrap_or_else(|| format!("<name#{idx}>"));
    if extra > 0 {
        format!("{base}_{}", extra - 1)
    } else {
        base
    }
}

/// The global name batch plus the `FScriptObjectEntry` table from `global.ucas`.
pub struct GlobalNames {
    pub names: Vec<String>,
    script: HashMap<u64, (String, u64)>,
}

impl GlobalNames {
    /// Read `global.ucas`: a name batch, then `FScriptObjectEntry` records (32 B each) mapping a
    /// global index to a name and its outer index.
    pub fn read(paks: &Path) -> Result<Self> {
        let raw = std::fs::read(paks.join("global.ucas"))
            .with_context(|| format!("reading {}/global.ucas", paks.display()))?;
        let (names, mut p) = parse_name_batch(&raw, 0)?;
        if p + 4 > raw.len() {
            bail!("global.ucas: script-object count overruns");
        }
        let count = u32le(&raw, p) as usize;
        p += 4;
        if p + count * 32 > raw.len() {
            bail!("global.ucas: script-object table out of range");
        }
        let mut script: HashMap<u64, (String, u64)> = HashMap::with_capacity(count);
        for i in 0..count {
            let at = p + i * 32;
            let ni = (u32le(&raw, at) & 0x3FFF_FFFF) as usize;
            let extra = u32le(&raw, at + 4);
            let global_index = u64le(&raw, at + 8);
            let outer = u64le(&raw, at + 16);
            let name = names.get(ni).cloned().unwrap_or_default();
            let name = if extra > 0 {
                format!("{name}_{}", extra - 1)
            } else {
                name
            };
            script.insert(global_index, (name, outer));
        }
        Ok(Self { names, script })
    }

    /// Resolve a script-object index through its outer chain into a dotted path.
    pub fn path_of(&self, idx: u64) -> String {
        let mut parts = Vec::new();
        let mut cur = idx;
        for _ in 0..64 {
            match self.script.get(&cur) {
                Some((name, outer)) => {
                    parts.push(name.clone());
                    if *outer == 0 {
                        break;
                    }
                    cur = *outer;
                }
                None => break,
            }
        }
        if parts.is_empty() {
            return format!("<script {idx:#x}>");
        }
        parts.reverse();
        parts.join(".")
    }
}

/// One `FExportMapEntry`.
pub struct Export {
    /// `CookedSerialOffset`, a package-internal offset (see [`ZenPackage::export_data`]).
    pub cooked_offset: u64,
    /// `CookedSerialSize`.
    pub cooked_size: u64,
    pub name: String,
    pub class: String,
}

/// A parsed Zen package chunk.
pub struct ZenPackage {
    pub header_size: usize,
    pub cooked_header_size: usize,
    pub exports: Vec<Export>,
    pub local_names: Vec<String>,
    /// Logical start of each export's cooked bytes within the chunk (physical layout).
    pub export_starts: Vec<usize>,
}

impl ZenPackage {
    /// Parse the summary, name batch, and export map of one package chunk.
    pub fn parse(chunk: &[u8], global: &GlobalNames) -> Result<Self> {
        if chunk.len() < 44 {
            bail!("chunk too small for a Zen summary");
        }
        let has_versioning_info = u32le(chunk, 0);
        let header_size = u32le(chunk, 4) as usize;
        let cooked_header_size = u32le(chunk, 20) as usize;
        let import_off = i32le(chunk, 28) as usize;
        let export_off = i32le(chunk, 32) as usize;
        let bundle_off = i32le(chunk, 36) as usize;
        if !(0 < header_size && header_size <= chunk.len()) {
            bail!("bad header size {header_size}");
        }

        // The name batch follows the summary, past the versioning info when present.
        let mut p = 44;
        if has_versioning_info != 0 {
            p += 16;
            let n = i32le(chunk, p) as usize;
            p += 4 + n * 20;
        }
        let (local_names, _) = parse_name_batch(chunk, p)?;

        let n_exports = bundle_off.saturating_sub(export_off) / 72;
        if n_exports == 0 || n_exports > 100_000 {
            bail!("implausible export count {n_exports}");
        }
        let mut exports = Vec::with_capacity(n_exports);
        let mut export_starts = Vec::with_capacity(n_exports);
        for i in 0..n_exports {
            let at = export_off + i * 72;
            let cooked_offset = u64le(chunk, at);
            let cooked_size = u64le(chunk, at + 8);
            let name = mapped_name(chunk, at + 16, &local_names, &global.names);
            let class_index = u64le(chunk, at + 32);
            let class = match class_index >> 62 {
                1 => global.path_of(class_index),
                0 => format!("<export {class_index}>"),
                2 => format!("<package import {class_index}>"),
                _ => "<null>".into(),
            };
            // Physical start: the header plus the export's offset within the cooked region.
            // `cooked_header_size` is the size of the header block that precedes that region.
            let start = header_size
                .checked_add(cooked_offset as usize)
                .and_then(|v| v.checked_sub(cooked_header_size));
            exports.push(Export {
                cooked_offset,
                cooked_size,
                name,
                class,
            });
            export_starts.push(start.unwrap_or(usize::MAX));
        }
        let _ = import_off;
        Ok(Self {
            header_size,
            cooked_header_size,
            exports,
            local_names,
            export_starts,
        })
    }

    /// The cooked bytes of export `i` within `chunk`, when the computed range is valid.
    pub fn export_data<'a>(&self, chunk: &'a [u8], i: usize) -> Option<&'a [u8]> {
        let start = *self.export_starts.get(i)?;
        let size = self.exports.get(i)?.cooked_size as usize;
        (start != usize::MAX && start + size <= chunk.len()).then(|| &chunk[start..start + size])
    }

    /// Candidate cooked-byte ranges for export `i`, most likely first.
    ///
    /// The first candidate is the [`export_data`](Self::export_data) formula (the export's offset
    /// within the cooked region). One shipped package (the Ultimate Bunny head) lays its exports
    /// out in the *reverse* of the export-map order — the mesh is physically before the morph
    /// export even though its recorded offset is larger — so a second, reverse-order candidate is
    /// offered; a caller that must not guess tries each and keeps the range that parses
    /// (`MESH-FORMAT.md`).
    pub fn export_data_variants<'a>(&self, chunk: &'a [u8], i: usize) -> Vec<&'a [u8]> {
        let Some(export) = self.exports.get(i) else {
            return Vec::new();
        };
        let size = export.cooked_size as usize;
        let forward = self.export_starts.get(i).copied().unwrap_or(usize::MAX);
        let reverse = self.header_size
            + self.exports[i + 1..]
                .iter()
                .map(|e| e.cooked_size as usize)
                .sum::<usize>();
        let mut out: Vec<&'a [u8]> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        for start in [forward, reverse] {
            if start != usize::MAX && start + size <= chunk.len() && !starts.contains(&start) {
                starts.push(start);
                out.push(&chunk[start..start + size]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_batch(names: &[&str]) -> Vec<u8> {
        let mut strings = Vec::new();
        for n in names {
            strings.extend_from_slice(n.as_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // hash version
        out.extend_from_slice(&0u32.to_le_bytes()); // (padding to the 16-byte header)
        for _ in names {
            out.extend_from_slice(&0u64.to_le_bytes()); // per-entry hash
        }
        for n in names {
            let len = n.len();
            out.push(((len >> 8) & 0x7f) as u8);
            out.push((len & 0xff) as u8);
        }
        out.extend_from_slice(&strings);
        out
    }

    #[test]
    fn parses_a_small_name_batch() {
        let b = name_batch(&["Alpha", "Beta"]);
        let (names, end) = parse_name_batch(&b, 0).unwrap();
        assert_eq!(names, vec!["Alpha", "Beta"]);
        assert_eq!(end, b.len());
    }

    #[test]
    fn mapped_name_uses_extra_suffix_and_local_table() {
        let local = vec!["Root".to_string(), "Child".to_string()];
        let mut b = Vec::new();
        b.extend_from_slice(&1u32.to_le_bytes()); // index 1, type 0 -> local
        b.extend_from_slice(&3u32.to_le_bytes()); // extra 3 -> suffix _2
        assert_eq!(mapped_name(&b, 0, &local, &[]), "Child_2");
    }

    /// Parse a real package where this machine has data and a key. Chunk 47475 is the Ultimate
    /// Bunny body `USkeletalMesh` package on this install; its mesh export size is the value the
    /// research parser validated to the exact export boundary (`MESH-FORMAT.md`).
    #[test]
    fn body_mesh_package_exposes_its_mesh_export() {
        let Some(paks) = crate::paks_dir() else {
            spdesc_game_core::skipped("no game data found", &spdesc_game_core::candidates());
            return;
        };
        let Some(key) = crate::key::aes_key() else {
            spdesc_game_core::skipped(
                "no container key (spdesc-config/keys.json)",
                &crate::key::config_root().into_iter().collect::<Vec<_>>(),
            );
            return;
        };
        let mut store = crate::container::IoStore::open(&paks, key).expect("open IoStore");
        let chunk = store.read_chunk(47475).expect("read body mesh chunk");
        let global = GlobalNames::read(&paks).expect("read global.ucas");
        let package = ZenPackage::parse(&chunk, &global).expect("parse body mesh package");

        let index = package
            .exports
            .iter()
            .position(|e| e.name == "PC_004_U_CMN_BODY_001")
            .expect("body mesh export present");
        assert_eq!(package.exports[index].cooked_size, 12_376_061);
        assert_eq!(
            package.export_data(&chunk, index).map(<[u8]>::len),
            Some(12_376_061),
            "the mesh export's cooked range lies inside the chunk"
        );
    }
}
