//! The IoStore container: the `.utoc` tables and the `.ucas` blocks.
//!
//! A chunk id resolves through the TOC's offset/length table to a byte range of the `.ucas`; that
//! range is cut on 64 KiB block boundaries, each block is AES-256-ECB decrypted (the TOC records
//! whether the store is encrypted) and, when the block records a compression method, run through
//! the Oodle decompressor to exactly the uncompressed length the TOC stores.
//!
//! Layout and field offsets follow CUE4Parse `FIoStoreTocResource`
//! (`FIoStoreTocHeader`, `FIoOffsetAndLength`, `FIoStoreTocCompressedBlockEntry`).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use aes::Aes256;
use aes::cipher::{BlockDecrypt, KeyInit};
use anyhow::{Context, Result, bail};

/// FIoStoreTocHeader: 16-byte magic, version byte, then the counts and tables.
const TOC_MAGIC: &[u8; 16] = b"-==--==--==--==-";
const TOC_VERSION: u8 = 5;

struct Toc {
    chunks: usize,
    blocks: usize,
    block_size: u64,
    ids_at: usize,
    offlen_at: usize,
    block_table_at: usize,
    encrypted: bool,
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// FIoOffsetAndLength: 5-byte big-endian offset and 5-byte big-endian length.
fn be40(b: &[u8], at: usize) -> u64 {
    b[at..at + 5]
        .iter()
        .fold(0u64, |acc, x| (acc << 8) | u64::from(*x))
}

/// FIoStoreTocCompressedBlockEntry: physical offset (5 B LE), compressed size (3 B LE),
/// uncompressed size (3 B LE), compression method (1 B).
fn block_at(b: &[u8], at: usize) -> (u64, u32, u32, u8) {
    let phys = u64::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3], b[at + 4], 0, 0, 0]);
    let csize = u32::from_le_bytes([b[at + 5], b[at + 6], b[at + 7], 0]);
    let usize_ = u32::from_le_bytes([b[at + 8], b[at + 9], b[at + 10], 0]);
    (phys, csize, usize_, b[at + 11])
}

fn parse_toc(b: &[u8]) -> Result<Toc> {
    if b.len() < 144 || &b[0..16] != TOC_MAGIC || b[16] != TOC_VERSION {
        bail!("not a v{TOC_VERSION} IoStore TOC");
    }
    let chunks = u32le(b, 24) as usize;
    let blocks = u32le(b, 28) as usize;
    let block_size = u64::from(u32le(b, 44));
    let flags = b[80];
    let seeds = u32le(b, 84) as usize;
    let overflow = u32le(b, 96) as usize;
    let ids_at = 144;
    let offlen_at = ids_at + chunks * 12;
    let block_table_at = offlen_at + chunks * 10 + seeds * 4 + overflow * 4;
    if blocks == 0 || block_table_at + blocks * 12 > b.len() {
        bail!("TOC tables out of range");
    }
    Ok(Toc {
        chunks,
        blocks,
        block_size,
        ids_at,
        offlen_at,
        block_table_at,
        encrypted: flags & 2 != 0,
    })
}

impl Toc {
    fn chunk_range(&self, b: &[u8], i: usize) -> (u64, u64, u8) {
        let at = self.offlen_at + i * 10;
        (be40(b, at), be40(b, at + 5), b[self.ids_at + i * 12 + 11])
    }
}

fn aes_ecb_decrypt(key: &[u8; 32], data: &mut [u8]) -> Result<()> {
    let cipher = Aes256::new_from_slice(key).map_err(|e| anyhow::anyhow!("bad AES key: {e:?}"))?;
    if !data.len().is_multiple_of(16) {
        bail!("ECB input not 16-aligned: {}", data.len());
    }
    for block in data.as_chunks_mut::<16>().0 {
        cipher.decrypt_block(block.into());
    }
    Ok(())
}

/// An open IoStore bundle. Holds the `.ucas` file and the TOC tables.
pub struct IoStore {
    toc: Toc,
    utoc: Vec<u8>,
    ucas: File,
    key: [u8; 32],
}

impl IoStore {
    /// Open `<paks>/M1-Windows.utoc` and `.ucas` with the given container key.
    pub fn open(paks: &Path, key: [u8; 32]) -> Result<Self> {
        let utoc = std::fs::read(paks.join("M1-Windows.utoc"))
            .with_context(|| format!("reading {}", paks.display()))?;
        let toc = parse_toc(&utoc)?;
        let ucas = File::open(paks.join("M1-Windows.ucas"))
            .with_context(|| format!("opening {}/M1-Windows.ucas", paks.display()))?;
        Ok(Self {
            toc,
            utoc,
            ucas,
            key,
        })
    }

    /// Number of chunks the TOC indexes.
    pub fn chunk_count(&self) -> usize {
        self.toc.chunks
    }

    /// The logical (decompressed) byte length of a chunk, or `None` when out of range.
    pub fn chunk_len(&self, id: usize) -> Option<u64> {
        if id >= self.toc.chunks {
            return None;
        }
        Some(self.toc.chunk_range(&self.utoc, id).1)
    }

    fn read_at(&mut self, phys: u64, len: usize) -> Result<Vec<u8>> {
        self.ucas.seek(SeekFrom::Start(phys))?;
        let mut v = vec![0u8; len];
        self.ucas.read_exact(&mut v)?;
        Ok(v)
    }

    /// The container's directory index: file path → chunk id. The index sits after the block table
    /// (and the optional integrity-hash section) and is itself AES-ECB encrypted when the store is,
    /// so it is decrypted with the same container key.
    pub fn directory_index(&self) -> Result<HashMap<String, u32>> {
        let b = &self.utoc;
        let version = b[16];
        let codec_count = u32le(b, 36) as usize;
        let codec_len = u32le(b, 40) as usize;
        let dir_size = u32le(b, 48) as usize;
        let flags = b[80];
        let seeds = u32le(b, 84) as usize;
        let overflow = u32le(b, 96) as usize;
        let mut at = 144;
        at += self.toc.chunks * 12;
        at += self.toc.chunks * 10;
        at += seeds * 4;
        at += overflow * 4;
        at += self.toc.blocks * 12;
        let iv_count = if version < 10 {
            0
        } else {
            u32le(b, 100) as usize
        };
        at += iv_count * 16;
        at += codec_count * codec_len;
        if flags & 4 != 0 {
            if at + 4 > b.len() {
                bail!("TOC hash section overruns");
            }
            let hash_size = u32le(b, at) as usize;
            at += 4 + hash_size + hash_size + 20 * self.toc.blocks;
        }
        if at + dir_size > b.len() {
            bail!("directory index out of range");
        }
        let raw = &b[at..at + dir_size];
        let dec = if self.toc.encrypted {
            let mut v = raw.to_vec();
            let padded = v.len().div_ceil(16) * 16;
            v.resize(padded, 0);
            aes_ecb_decrypt(&self.key, &mut v)?;
            v.truncate(raw.len());
            v
        } else {
            raw.to_vec()
        };
        parse_directory_index(&dec)
    }

    /// Extract one chunk: the blocks covering its range, each decrypted and decompressed, sliced
    /// to the logical range.
    pub fn read_chunk(&mut self, id: usize) -> Result<Vec<u8>> {
        if id >= self.toc.chunks {
            bail!("chunk {id} out of range ({} chunks)", self.toc.chunks);
        }
        let (o, l, _ctype) = self.toc.chunk_range(&self.utoc, id);
        if l == 0 {
            bail!("chunk {id} is empty");
        }
        let bs = self.toc.block_size;
        let first = o / bs;
        let last = (o + l - 1) / bs;
        if last >= self.toc.blocks as u64 {
            bail!(
                "chunk {id}: block {last} out of range of {}",
                self.toc.blocks
            );
        }
        let mut logical = Vec::with_capacity(((last - first + 1) * bs) as usize);
        for j in first..=last {
            let at = self.toc.block_table_at + j as usize * 12;
            let (phys, csize, usize_, method) = block_at(&self.utoc, at);
            let store = if self.toc.encrypted {
                // The on-disk AES unit is 16-aligned; padding is dropped before decompression.
                let aligned = (csize as usize + 15) & !15;
                let mut padded = self.read_at(phys, aligned)?;
                aes_ecb_decrypt(&self.key, &mut padded)?;
                padded.truncate(csize as usize);
                padded
            } else {
                self.read_at(phys, csize as usize)?
            };
            let mut out = vec![0u8; usize_ as usize];
            match method {
                0 => out.copy_from_slice(&store[..usize_ as usize]),
                1 => {
                    let mut ex = oozextract::Extractor::new();
                    let n = ex
                        .read_from_slice(&store, &mut out)
                        .with_context(|| format!("chunk {id} block {j}: Oodle decode"))?;
                    if n != usize_ as usize {
                        bail!("chunk {id} block {j}: Oodle wrote {n}, TOC says {usize_}");
                    }
                }
                m => bail!("chunk {id} block {j}: unknown compression method {m}"),
            }
            logical.extend_from_slice(&out);
        }
        let start = (o - first * bs) as usize;
        Ok(logical[start..start + l as usize].to_vec())
    }
}

/// An `FString` at `at`: a signed length (negative = UTF-16) then the bytes. Returns the string and
/// the offset just past it.
fn read_fstring(b: &[u8], at: usize) -> Result<(String, usize)> {
    if at + 4 > b.len() {
        bail!("FString length overruns");
    }
    let n = i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let mut p = at + 4;
    if n == 0 {
        return Ok((String::new(), p));
    }
    if n < 0 {
        let chars = (-n) as usize;
        if p + chars * 2 > b.len() {
            bail!("UTF-16 FString overruns");
        }
        let s = String::from_utf16_lossy(
            &(0..chars)
                .map(|k| u16::from_le_bytes([b[p + k * 2], b[p + k * 2 + 1]]))
                .collect::<Vec<_>>(),
        );
        p += chars * 2;
        Ok((s, p))
    } else {
        let bytes = n as usize;
        if p + bytes > b.len() {
            bail!("FString overruns");
        }
        let end = bytes.saturating_sub(1);
        Ok((
            String::from_utf8_lossy(&b[p..p + end]).into_owned(),
            p + bytes,
        ))
    }
}

/// The `FIoDirectoryIndex`: a mount point, directory and file entry arrays, and a shared string
/// pool. Each file entry's `user_data` is the chunk id the path resolves to.
fn parse_directory_index(dec: &[u8]) -> Result<HashMap<String, u32>> {
    const INVALID: u32 = 0xFFFF_FFFF;
    let (_mount, mut at) = read_fstring(dec, 0)?;

    if at + 4 > dec.len() {
        bail!("directory index: directory count overruns");
    }
    let num_dirs = i32::from_le_bytes([dec[at], dec[at + 1], dec[at + 2], dec[at + 3]]) as usize;
    at += 4;
    if at + num_dirs * 16 > dec.len() {
        bail!("directory index: directory array out of range");
    }
    let dirs: Vec<(u32, u32, u32, u32)> = (0..num_dirs)
        .map(|i| {
            let o = at + i * 16;
            (
                u32le(dec, o),
                u32le(dec, o + 4),
                u32le(dec, o + 8),
                u32le(dec, o + 12),
            )
        })
        .collect();
    at += num_dirs * 16;

    let num_files = i32::from_le_bytes([dec[at], dec[at + 1], dec[at + 2], dec[at + 3]]) as usize;
    at += 4;
    if at + num_files * 12 > dec.len() {
        bail!("directory index: file array out of range");
    }
    let files: Vec<(u32, u32, u32)> = (0..num_files)
        .map(|i| {
            let o = at + i * 12;
            (u32le(dec, o), u32le(dec, o + 4), u32le(dec, o + 8))
        })
        .collect();
    at += num_files * 12;

    let num_strings = i32::from_le_bytes([dec[at], dec[at + 1], dec[at + 2], dec[at + 3]]) as usize;
    at += 4;
    let mut strings = Vec::with_capacity(num_strings);
    for _ in 0..num_strings {
        let (s, next) = read_fstring(dec, at)?;
        strings.push(s);
        at = next;
    }

    let name_of = |i: u32| -> &str {
        strings
            .get(i as usize)
            .map(String::as_str)
            .unwrap_or_default()
    };
    let mut index = HashMap::new();
    // Depth-first over the directory tree; an explicit stack keeps deep trees off the call stack.
    let mut stack: Vec<(u32, String)> = vec![(0, String::new())];
    let mut guard = 0usize;
    while let Some((mut dir, prefix)) = stack.pop() {
        while dir != INVALID && (dir as usize) < dirs.len() {
            guard += 1;
            if guard > 5_000_000 {
                bail!("directory index: traversal did not terminate");
            }
            let (name_i, first_child, next_sib, first_file) = dirs[dir as usize];
            let path = format!("{prefix}{}", name_of(name_i));
            let mut f = first_file;
            while f != INVALID && (f as usize) < files.len() {
                let (fname_i, next_file, user_data) = files[f as usize];
                index.insert(format!("{path}{}", name_of(fname_i)), user_data);
                f = next_file;
            }
            if first_child != INVALID {
                stack.push((first_child, format!("{path}/")));
            }
            dir = next_sib;
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_non_toc_buffer() {
        assert!(parse_toc(&[0u8; 200]).is_err());
    }

    #[test]
    fn be40_reads_big_endian_five_bytes() {
        assert_eq!(be40(&[0x01, 0x02, 0x03, 0x04, 0x05], 0), 0x0001_0203_0405);
    }

    /// Read the container end-to-end (TOC → AES-ECB → Oodle) where this machine has data and a
    /// key. Asserts the extracted bytes match the length the TOC records, which exercises both the
    /// AES unit and the Oodle framing without depending on chunk contents.
    #[test]
    fn extracts_a_chunk_to_its_recorded_length() {
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
        let mut store = IoStore::open(&paks, key).expect("open IoStore");
        assert!(store.chunk_count() > 0);
        // Chunk 4 is a small single-block package; its logical length is TOC-recorded.
        let want = store.chunk_len(4).expect("chunk 4 length");
        let got = store.read_chunk(4).expect("read chunk 4");
        assert_eq!(got.len() as u64, want);
    }

    /// The directory index resolves the same paths the research index recorded, and the chunk it
    /// names really does hold the body mesh package with its validated export size.
    #[test]
    fn directory_index_resolves_the_body_mesh_package() {
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
        let mut store = IoStore::open(&paks, key).expect("open IoStore");
        let index = store.directory_index().expect("directory index");
        assert!(index.len() > 100_000, "indexed {} paths", index.len());

        let mesh_path =
            "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/001PC_004_U_CMN_BODY_001.uasset";
        let chunk = *index
            .get(mesh_path)
            .unwrap_or_else(|| panic!("{mesh_path} is indexed"));
        assert!(index.contains_key(
            "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Body_C.uasset"
        ));
        assert!(index.contains_key(
            "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Body_C.ubulk"
        ));

        let data = store.read_chunk(chunk as usize).expect("read mesh chunk");
        let global = crate::zen::GlobalNames::read(&paks).expect("global.ucas");
        let package = crate::zen::ZenPackage::parse(&data, &global).expect("parse package");
        let export = package
            .exports
            .iter()
            .find(|e| e.name == "PC_004_U_CMN_BODY_001")
            .expect("mesh export");
        assert_eq!(export.cooked_size, 12_376_061);
    }
}
