//! Reading a file out of Modern Warfare 2's `.iwd` archives.
//!
//! `<MW2_DATA>/main/iw_*.iwd` are ZIP archives, and the big ones are hundreds of megabytes, so
//! nothing here reads an archive whole: the reader seeks to the central directory, then to the one
//! entry it wants. Most of what is in the archives is not what the game plays — the 18640 sounds
//! under `Sound/` are dialogue, music and ambience (`docs/research/MW2-AUDIO.md`) — but two things
//! are needed from them: the audio a *streamed* sound alias names (the level-up cue is one), and
//! the interface art, which ships only as `.iwi` files (`images/cardicon_rank_*.iwi`).
//!
//! The entries are usually stored rather than deflated, so a stored entry is handed back as it is.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::open::Error;

/// How far back the end-of-central-directory record can sit, with its maximum comment.
const EOCD_WINDOW: u64 = 0xffff + 22;

/// The archives under `<MW2_DATA>/main`, in name order.
pub fn archives(install: &Path) -> Vec<PathBuf> {
    let main = install.join("main");
    let Ok(entries) = std::fs::read_dir(&main) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("iwd"))
        })
        .collect();
    out.sort();
    out
}

/// The entry a streamed sound alias names: the game keeps them under `Sound/`.
pub fn streamed_sound_entry(directory: &str, file: &str) -> String {
    if directory.is_empty() {
        format!("Sound/{file}")
    } else {
        format!("Sound/{directory}/{file}")
    }
}

/// Three of the names `mp/rankicontable.csv` uses for a rank's insignia are spelled differently in
/// the art files. Measured against the archives: every other name matches as it stands.
const RANK_ICON_SPELLINGS: [(&str, &str); 3] = [
    // Sergeant First Class.
    ("rank_sfc1", "rank_sgtfc1"),
    // First Sergeant.
    ("rank_1stsgt1", "rank_fsgt1"),
    // The top rank, whose art has no trailing digit.
    ("rank_comm1", "rank_comm"),
];

/// The archive entry holding a rank icon, which `mp/rankicontable.csv` names (`rank_pvt1`).
pub fn rank_icon_entry(icon_name: &str) -> String {
    let name = RANK_ICON_SPELLINGS
        .iter()
        .find(|(table, _)| *table == icon_name)
        .map_or(icon_name, |(_, art)| *art);
    format!("images/cardicon_{name}.iwi")
}

/// The archive entry holding a prestige icon.
///
/// The icon table names these `rank_prestige<N>`, but the art is `cardicon_prestige_<N>.iwi` — a
/// different shape from the rank icons, which are the table's name with `cardicon_` in front.
pub fn prestige_icon_entry(prestige: u32) -> String {
    format!("images/cardicon_prestige_{prestige}.iwi")
}

/// Read `entry` from the first archive under `install` that holds it, or `None`.
///
/// The name is matched case-insensitively: the archives do not agree on the case of a path.
pub fn read_entry(install: &Path, entry: &str) -> Option<Vec<u8>> {
    read_many(install, &[entry.to_owned()])
        .into_iter()
        .next()
        .map(|(_, bytes)| bytes)
}

/// Read several entries at once, each from the first archive that holds it.
///
/// One archive is opened once for all the entries it holds, which is what keeps decoding a few
/// dozen small icons out of the hundreds of megabytes a single archive can be.
pub fn read_many(install: &Path, entries: &[String]) -> HashMap<String, Vec<u8>> {
    let mut wanted: HashMap<String, Vec<&String>> = HashMap::new();
    for entry in entries {
        wanted
            .entry(entry.to_ascii_lowercase())
            .or_default()
            .push(entry);
    }
    let mut found = HashMap::new();
    for archive in archives(install) {
        if wanted.is_empty() {
            break;
        }
        let Ok(mut reader) = Archive::open(&archive) else {
            continue;
        };
        let Some(records) = reader.records() else {
            continue;
        };
        for record in records {
            let lower = record.name.to_ascii_lowercase();
            let Some(names) = wanted.get(&lower) else {
                continue;
            };
            let Ok(bytes) = reader.entry_bytes(&record) else {
                continue;
            };
            for name in names.clone() {
                found.insert(name.clone(), bytes.clone());
            }
            wanted.remove(&lower);
        }
    }
    found
}

/// Read `entry` from one archive.
pub fn read_from(archive: &Path, entry: &str) -> Result<Option<Vec<u8>>, Error> {
    let mut reader = Archive::open(archive)?;
    let Some(records) = reader.records() else {
        return Ok(None);
    };
    for record in records {
        if record.name.eq_ignore_ascii_case(entry) {
            return reader.entry_bytes(&record).map(Some);
        }
    }
    Ok(None)
}

/// One central-directory record: where an entry's data is and how it is stored.
#[derive(Debug, Clone)]
struct Record {
    name: String,
    method: u16,
    compressed_size: usize,
    uncompressed_size: usize,
    local_offset: u64,
}

/// An archive open for seeking.
struct Archive {
    file: File,
}

impl Archive {
    fn open(path: &Path) -> Result<Archive, Error> {
        Ok(Archive {
            file: File::open(path)?,
        })
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>, Error> {
        self.file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0u8; len];
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// Walk the central directory. `None` when the file is not a readable ZIP.
    fn records(&mut self) -> Option<Vec<Record>> {
        let len = self.file.metadata().ok()?.len();
        let window = len.min(EOCD_WINDOW) as usize;
        let tail = self.read_at(len - window as u64, window).ok()?;
        let eocd = (0..=tail.len().saturating_sub(4))
            .rev()
            .find(|&at| &tail[at..at + 4] == b"PK\x05\x06")?;
        let u16_at = |o: usize| -> Option<u16> {
            tail.get(eocd + o..eocd + o + 2)?
                .try_into()
                .ok()
                .map(u16::from_le_bytes)
        };
        let u32_at = |o: usize| -> Option<u32> {
            tail.get(eocd + o..eocd + o + 4)?
                .try_into()
                .ok()
                .map(u32::from_le_bytes)
        };
        let count = u16_at(10)? as usize;
        let directory_size = u32_at(12)? as usize;
        let directory_offset = u64::from(u32_at(16)?);
        // The directory states its own size, so it can be read in one seek rather than walked in
        // chunks — which matters for an archive with thousands of entries.
        let directory = self.read_at(directory_offset, directory_size).ok()?;

        let mut out = Vec::with_capacity(count);
        let mut at = 0usize;
        for _ in 0..count {
            // The slice runs to the end of the directory, not to the end of this record: the
            // record's name and the rest of the directory follow its 46-byte header.
            let cell = directory.get(at..)?;
            if &cell[0..4] != b"PK\x01\x02" {
                return None;
            }
            let rd16 = |o: usize| u16::from_le_bytes(cell[o..o + 2].try_into().unwrap());
            let rd32 = |o: usize| u32::from_le_bytes(cell[o..o + 4].try_into().unwrap());
            let name_len = rd16(28) as usize;
            let extra_len = rd16(30) as usize;
            let comment_len = rd16(32) as usize;
            let name = cell.get(46..46 + name_len)?;
            out.push(Record {
                name: String::from_utf8_lossy(name).into_owned(),
                method: rd16(10),
                compressed_size: rd32(20) as usize,
                uncompressed_size: rd32(24) as usize,
                local_offset: u64::from(rd32(42)),
            });
            at = at.checked_add(46 + name_len + extra_len + comment_len)?;
        }
        Some(out)
    }

    /// An entry's bytes: seeked to, and inflated when the archive deflated it.
    fn entry_bytes(&mut self, record: &Record) -> Result<Vec<u8>, Error> {
        let header = self.read_at(record.local_offset, 30)?;
        if header.get(0..4) != Some(b"PK\x03\x04") {
            return Err(Error::Archive(format!(
                "`{}` has no local header",
                record.name
            )));
        }
        let name_len = u16::from_le_bytes([header[26], header[27]]) as usize;
        let extra_len = u16::from_le_bytes([header[28], header[29]]) as usize;
        let start = record.local_offset + 30 + name_len as u64 + extra_len as u64;
        let data = self.read_at(start, record.compressed_size)?;
        match record.method {
            0 => Ok(data),
            8 => inflate_raw(&data, record.uncompressed_size).map_err(Error::Archive),
            method => Err(Error::Archive(format!(
                "`{}` uses compression method {method}",
                record.name
            ))),
        }
    }
}

/// A raw deflate stream, which is what a ZIP entry carries (no zlib wrapper).
fn inflate_raw(bytes: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut decoder = Decompress::new(false);
    let mut out = Vec::with_capacity(expected);
    loop {
        out.reserve(expected.max(1024));
        let before = (decoder.total_in(), decoder.total_out());
        let status = decoder
            .decompress_vec(
                &bytes[decoder.total_in() as usize..],
                &mut out,
                FlushDecompress::None,
            )
            .map_err(|e| e.to_string())?;
        if status == Status::StreamEnd {
            return Ok(out);
        }
        if before == (decoder.total_in(), decoder.total_out()) {
            return Err("truncated deflate stream".into());
        }
    }
}
