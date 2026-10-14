//! `UTexture2D` → mip 0 RGBA.
//!
//! Cooked TFD textures are *virtual*: the classic mip array is empty and every pixel lives in a
//! separate `.ubulk` chunk as Morton-ordered, border-padded block-compressed tiles. This module
//! reads the export (property block, the cooked pixel-format loop, `FTexturePlatformData`,
//! `FVirtualTextureBuiltData`), resolves the `.ubulk` sibling through the container directory index,
//! assembles mip 0 from its tiles, and decodes BC1/BC3 to RGBA.
//!
//! The layout and the addressing rules were established byte-for-byte by the research parser
//! (`tex-poc.py`, `TEX-FORMAT.md`) on the Ultimate Bunny body and PartA textures; this is the
//! runtime port, validated against those PNGs.

use std::collections::HashMap;

use anyhow::{Context, Result, bail};

use crate::container::IoStore;
use crate::property::{Ctx, Names, Natives, Reader, Schema, read_struct};
use crate::zen::{GlobalNames, ZenPackage};

/// A decoded image: mip 0, tightly packed RGBA8.
#[derive(Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Texture {
    /// One pixel as `[r, g, b, a]`, when in range.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let o = ((y as usize) * self.width as usize + x as usize) * 4;
        self.rgba.get(o..o + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }
}

/// One `FVirtualTextureDataChunk`.
struct DataChunk {
    size_in_bytes: u32,
}

/// One `FVirtualTextureTileOffsetData`.
struct TileOffsetData {
    width: u32,
    height: u32,
}

/// The `FVirtualTextureBuiltData` fields mip-0 assembly needs.
struct VirtualTexture {
    tile_size: u32,
    tile_border: u32,
    layer_data: Vec<u32>,
    chunk_per_mip: Vec<u32>,
    base_per_mip: Vec<u32>,
    tile_offset_data: Vec<TileOffsetData>,
    layer_types: Vec<String>,
    chunks: Vec<DataChunk>,
}

/// `FTexturePlatformData`.
struct PlatformData {
    width: i32,
    height: i32,
    vt: Option<VirtualTexture>,
}

/// Decode `uasset_path` (a `/M1/...` container path) into a [`Texture`], resolving its `.ubulk`
/// sibling through `index`. `natives` must include the texture property chain's custom structs
/// (`IntPoint`, `Guid`, `LinearColor`).
pub fn load(
    store: &mut IoStore,
    index: &HashMap<String, u32>,
    global: &GlobalNames,
    schema: &Schema,
    natives: &Natives,
    uasset_path: &str,
) -> Result<Texture> {
    let chunk = *index
        .get(uasset_path)
        .with_context(|| format!("{uasset_path} is not in the directory index"))?;
    let uasset = store.read_chunk(chunk as usize)?;
    let package = ZenPackage::parse(&uasset, global)?;
    let eidx = package
        .exports
        .iter()
        .position(|e| e.class.ends_with("Texture2D") || e.name == export_stem(uasset_path))
        .or_else(|| (package.exports.len() == 1).then_some(0))
        .context("no Texture2D export in the package")?;
    let data = package
        .export_data(&uasset, eidx)
        .context("texture export data")?;

    let ctx = Ctx {
        schema,
        names: Names {
            local: &package.local_names,
            global: &global.names,
        },
        natives,
    };
    let mut r = Reader::new(data);
    read_struct(&mut r, "Texture2D", &ctx).context("Texture2D property block")?;

    let unknown = r.u32()?;
    if unknown != 0 {
        bail!("texture preamble: unknown u32 is {unknown}, expected 0");
    }
    let _strip1 = (r.u8()?, r.u8()?);
    let _strip2 = (r.u8()?, r.u8()?);
    if !r.bool32()? {
        bail!("texture is not cooked");
    }
    let serialize_mip_data = r.bool32()?;

    let mut platform: Option<PlatformData> = None;
    loop {
        let format = ctx.names.read(&mut r)?;
        if format == "None" {
            break;
        }
        let skip_base = r.offset();
        let skip = r.i64()?;
        let target = skip_base as i64 + skip;
        let plat = read_platform(&mut r, serialize_mip_data)?;
        if r.offset() as i64 != target {
            bail!(
                "platform data ended at {}, but the skip offset targets {target}",
                r.offset()
            );
        }
        platform = Some(plat);
    }
    if r.offset() != data.len() {
        bail!(
            "texture export ended at {} != {} bytes",
            r.offset(),
            data.len()
        );
    }
    let plat = platform.context("texture has no platform data")?;
    let vt = plat.vt.context("texture is not virtual")?;

    let ubulk_path = uasset_path.replace(".uasset", ".ubulk");
    let ubulk_chunk = *index
        .get(&ubulk_path)
        .with_context(|| format!("{ubulk_path} is not in the directory index"))?;
    let ubulk = store.read_chunk(ubulk_chunk as usize)?;

    assemble_mip0(&vt, plat.width, plat.height, &ubulk)
}

fn export_stem(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".uasset")
}

fn read_platform(r: &mut Reader, serialize_mip_data: bool) -> Result<PlatformData> {
    let derived = r.u8()?;
    if derived != 0 {
        bail!("derived-data platform payloads are unsupported");
    }
    r.skip(15)?; // UE5.2+ placeholder
    let width = r.i32()?;
    let height = r.i32()?;
    let packed = r.u32()?;
    let _format = r.fstring()?;
    if packed & (1 << 30) != 0 {
        let _ext_data = r.u32()?;
        let _tail = r.u32()?;
    }
    if packed & (1 << 29) != 0 {
        bail!("CPU-copy platform payloads are unsupported");
    }
    let _first_mip = r.i32()?;
    let mip_count = r.i32()?;
    if !(0..64).contains(&mip_count) {
        bail!("platform mip count {mip_count}");
    }
    for _ in 0..mip_count {
        if serialize_mip_data {
            let flags = r.u32()?;
            if flags & 0x40 != 0 {
                let _count = r.u64()?;
                let _size = r.u64()?;
            } else {
                let _count = r.i32()?;
                let _size = r.u32()?;
            }
            let _offset = r.i64()?;
        }
        let _size = (r.i32()?, r.i32()?, r.i32()?);
    }
    let is_virtual = r.bool32()?;
    let vt = if is_virtual {
        Some(read_virtual_texture(r)?)
    } else {
        None
    };
    Ok(PlatformData { width, height, vt })
}

fn read_u32_array(r: &mut Reader, what: &str) -> Result<Vec<u32>> {
    let n = r.i32()?;
    if !(0..1 << 20).contains(&n) {
        bail!("{what}: bad array count {n}");
    }
    (0..n).map(|_| r.u32()).collect()
}

fn read_virtual_texture(r: &mut Reader) -> Result<VirtualTexture> {
    if !r.bool32()? {
        bail!("virtual texture is not cooked");
    }
    let num_layers = r.u32()?;
    if !(1..=8).contains(&num_layers) {
        bail!("virtual texture layer count {num_layers}");
    }
    let _width_in_blocks = r.u32()?;
    let _height_in_blocks = r.u32()?;
    let tile_size = r.u32()?;
    let tile_border = r.u32()?;
    let layer_data = read_u32_array(r, "TileDataOffsetPerLayer")?;
    if layer_data.len() != num_layers as usize {
        bail!("TileDataOffsetPerLayer length != layers");
    }
    let num_mips = r.u32()?;
    let width = r.u32()?;
    let height = r.u32()?;
    let chunk_per_mip = read_u32_array(r, "ChunkIndexPerMip")?;
    let base_per_mip = read_u32_array(r, "BaseOffsetPerMip")?;
    if chunk_per_mip.len() != num_mips as usize || base_per_mip.len() != num_mips as usize {
        bail!("per-mip arrays != NumMips");
    }
    let tod_count = r.i32()?;
    if tod_count != num_mips as i32 {
        bail!("TileOffsetData count {tod_count} != NumMips {num_mips}");
    }
    let mut tile_offset_data = Vec::with_capacity(tod_count as usize);
    for _ in 0..tod_count {
        let tw = r.u32()?;
        let th = r.u32()?;
        let _max = r.u32()?;
        let _addresses = read_u32_array(r, "TileOffsetData.Addresses")?;
        let _offsets = read_u32_array(r, "TileOffsetData.Offsets")?;
        tile_offset_data.push(TileOffsetData {
            width: tw,
            height: th,
        });
    }
    let _tile_index_per_chunk = read_u32_array(r, "TileIndexPerChunk")?;
    let _tile_index_per_mip = read_u32_array(r, "TileIndexPerMip")?;
    let _tile_offset_in_chunk = read_u32_array(r, "TileOffsetInChunk")?;
    let layer_types = (0..num_layers)
        .map(|_| r.fstring())
        .collect::<Result<Vec<_>>>()?;
    for _ in 0..num_layers {
        for _ in 0..4 {
            let _ = r.f32()?; // FLinearColor fallback
        }
    }
    let nchunks = r.i32()?;
    if !(0 < nchunks && nchunks < 64) {
        bail!("virtual texture chunk count {nchunks}");
    }
    let mut chunks = Vec::with_capacity(nchunks as usize);
    for _ in 0..nchunks {
        r.skip(20)?; // FSHAHash
        let size_in_bytes = r.u32()?;
        let _codec_payload_size = r.u32()?;
        for _ in 0..num_layers {
            let _codec = r.u8()?;
            let _payload_offset = r.u32()?;
        }
        let _bulk_index = r.i32()?;
        chunks.push(DataChunk { size_in_bytes });
    }
    let _ = (width, height);
    Ok(VirtualTexture {
        tile_size,
        tile_border,
        layer_data,
        chunk_per_mip,
        base_per_mip,
        tile_offset_data,
        layer_types,
        chunks,
    })
}

fn bytes_per_pixel(layer_type: &str) -> Option<f64> {
    Some(match layer_type {
        "PF_DXT1" | "PF_BC4" => 0.5,
        "PF_DXT5" | "PF_DXT3" | "PF_BC5" | "PF_BC7" => 1.0,
        _ => return None,
    })
}

fn assemble_mip0(
    vt: &VirtualTexture,
    platform_width: i32,
    platform_height: i32,
    ubulk: &[u8],
) -> Result<Texture> {
    let layer0 = vt
        .layer_types
        .first()
        .context("virtual texture has no layers")?;
    let bpp = bytes_per_pixel(layer0)
        .with_context(|| format!("no size model for layer type {layer0}"))?;
    let phys = vt.tile_size + 2 * vt.tile_border;
    let tile_bytes = (f64::from(phys) * f64::from(phys) * bpp) as usize;
    if vt.layer_data[0] as usize != tile_bytes {
        bail!(
            "tile data size {} != computed {tile_bytes}",
            vt.layer_data[0]
        );
    }
    let mut offsets = Vec::with_capacity(vt.chunks.len());
    let mut acc = 0usize;
    for c in &vt.chunks {
        offsets.push(acc);
        acc += c.size_in_bytes as usize;
    }
    if acc != ubulk.len() {
        bail!(
            "virtual-texture chunks sum to {acc} != .ubulk length {}",
            ubulk.len()
        );
    }

    let tod = vt.tile_offset_data.first().context("no mip 0 tile data")?;
    let chunk_index = *vt.chunk_per_mip.first().context("no mip 0 chunk")? as usize;
    let base = vt.base_per_mip[0] as usize;
    let chunk_off = *offsets
        .get(chunk_index)
        .with_context(|| format!("mip 0 chunk {chunk_index} out of range"))?;

    let ts = vt.tile_size as usize;
    let border = vt.tile_border as usize;
    let width = tod.width as usize * ts;
    let height = tod.height as usize * ts;
    if platform_width > 0 && platform_width as usize != width {
        bail!("mip 0 width {width} != platform {platform_width}");
    }
    if platform_height > 0 && platform_height as usize != height {
        bail!("mip 0 height {height} != platform {platform_height}");
    }

    let decode: fn(&[u8], usize, usize) -> Result<Vec<u8>> = match layer0.as_str() {
        "PF_DXT1" | "PF_BC4" => decode_bc1_mip,
        _ => decode_bc3_mip,
    };

    let mut image = vec![0u8; width * height * 4];
    let mut decoded_any = false;
    for ty in 0..tod.height as usize {
        for tx in 0..tod.width as usize {
            // Address = Morton code with x in the even bits (LSB-first interleave).
            let addr = morton(tx as u32, ty as u32) as usize;
            let off = chunk_off + base + addr * tile_bytes;
            let blob = ubulk
                .get(off..off + tile_bytes)
                .with_context(|| format!("tile ({tx},{ty}) truncated at {off}"))?;
            let tile = decode(blob, phys as usize, phys as usize)?;
            decoded_any = true;
            for y in 0..ts {
                let row = (ty * ts + y) * width;
                let src = ((border + y) * phys as usize + border) * 4;
                image[(row + tx * ts) * 4..(row + tx * ts) * 4 + ts * 4]
                    .copy_from_slice(&tile[src..src + ts * 4]);
            }
        }
    }
    if !decoded_any {
        bail!("mip 0 has no tiles");
    }
    Ok(Texture {
        width: width as u32,
        height: height as u32,
        rgba: image,
    })
}

/// The Morton code with x in the even bits and y in the odd bits (LSB-first interleave).
fn morton(x: u32, y: u32) -> u32 {
    let mut r = 0u32;
    for i in 0..16 {
        r |= (((x >> i) & 1) << (2 * i)) | (((y >> i) & 1) << (2 * i + 1));
    }
    r
}

fn rgb565(v: u16) -> [u8; 4] {
    [
        ((v >> 11) as u32 * 255 / 31) as u8,
        (((v >> 5) & 0x3f) as u32 * 255 / 63) as u8,
        ((v & 0x1f) as u32 * 255 / 31) as u8,
        255,
    ]
}

fn lerp(a: &[u8; 4], b: &[u8; 4], n: u32, d: u32) -> [u8; 4] {
    let mut out = [0u8; 4];
    for i in 0..4 {
        out[i] = ((a[i] as u32 * (d - n) + b[i] as u32 * n) / d) as u8;
    }
    out
}

fn decode_bc1_mip(data: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    let (bx, by) = (w.div_ceil(4), h.div_ceil(4));
    if data.len() < bx * by * 8 {
        bail!("BC1 data {} < {}", data.len(), bx * by * 8);
    }
    let mut out = vec![0u8; w * h * 4];
    let mut at = 0usize;
    for j in 0..by {
        for i in 0..bx {
            let c0 = u16::from_le_bytes([data[at], data[at + 1]]);
            let c1 = u16::from_le_bytes([data[at + 2], data[at + 3]]);
            let bits = u32::from_le_bytes([data[at + 4], data[at + 5], data[at + 6], data[at + 7]]);
            at += 8;
            let p0 = rgb565(c0);
            let p1 = rgb565(c1);
            let pal = if c0 > c1 {
                [p0, p1, lerp(&p0, &p1, 1, 3), lerp(&p0, &p1, 2, 3)]
            } else {
                [p0, p1, lerp(&p0, &p1, 1, 2), [0, 0, 0, 0]]
            };
            for py in 0..4 {
                let y = j * 4 + py;
                if y >= h {
                    continue;
                }
                for px in 0..4 {
                    let x = i * 4 + px;
                    if x >= w {
                        continue;
                    }
                    let k = py * 4 + px;
                    let c = pal[((bits >> (2 * k)) & 3) as usize];
                    let o = (y * w + x) * 4;
                    out[o..o + 4].copy_from_slice(&c);
                }
            }
        }
    }
    Ok(out)
}

fn decode_bc3_mip(data: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    let (bx, by) = (w.div_ceil(4), h.div_ceil(4));
    if data.len() < bx * by * 16 {
        bail!("BC3 data {} < {}", data.len(), bx * by * 16);
    }
    let mut out = vec![0u8; w * h * 4];
    let mut at = 0usize;
    for j in 0..by {
        for i in 0..bx {
            let a0 = data[at] as u32;
            let a1 = data[at + 1] as u32;
            let mut abits = 0u64;
            for k in 0..6 {
                abits |= (data[at + 2 + k] as u64) << (8 * k);
            }
            let c0 = u16::from_le_bytes([data[at + 8], data[at + 9]]);
            let c1 = u16::from_le_bytes([data[at + 10], data[at + 11]]);
            let bits =
                u32::from_le_bytes([data[at + 12], data[at + 13], data[at + 14], data[at + 15]]);
            at += 16;
            let mut apal = [0u32; 8];
            apal[0] = a0;
            apal[1] = a1;
            if a0 > a1 {
                for k in 1..7 {
                    apal[k + 1] = ((7 - k as u32) * a0 + k as u32 * a1) / 7;
                }
            } else {
                for k in 1..5 {
                    apal[k + 1] = ((5 - k as u32) * a0 + k as u32 * a1) / 5;
                }
                apal[6] = 0;
                apal[7] = 255;
            }
            let p0 = rgb565(c0);
            let p1 = rgb565(c1);
            let pal = [p0, p1, lerp(&p0, &p1, 1, 3), lerp(&p0, &p1, 2, 3)];
            for py in 0..4 {
                let y = j * 4 + py;
                if y >= h {
                    continue;
                }
                for px in 0..4 {
                    let x = i * 4 + px;
                    if x >= w {
                        continue;
                    }
                    let k = py * 4 + px;
                    let mut c = pal[((bits >> (2 * k)) & 3) as usize];
                    c[3] = apal[((abits >> (3 * k)) & 7) as usize] as u8;
                    let o = (y * w + x) * 4;
                    out[o..o + 4].copy_from_slice(&c);
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::Natives;

    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    fn load_texture(path: &str) -> Option<Texture> {
        let Some(paks) = crate::paks_dir() else {
            spdesc_game_core::skipped("no game data found", &spdesc_game_core::candidates());
            return None;
        };
        let Some(key) = crate::key::aes_key() else {
            spdesc_game_core::skipped(
                "no container key (spdesc-config/keys.json)",
                &crate::key::config_root().into_iter().collect::<Vec<_>>(),
            );
            return None;
        };
        let Ok(schema) = crate::mapping::load() else {
            spdesc_game_core::skipped(
                "no property mapping (spdesc-config/mappings)",
                &crate::mapping::mapping_path()
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
            return None;
        };
        let mut natives = Natives::new();
        crate::skeletal::register_natives(&mut natives);
        let mut store = IoStore::open(&paks, key).expect("open IoStore");
        let index = store.directory_index().expect("directory index");
        let global = GlobalNames::read(&paks).expect("global.ucas");
        Some(load(&mut store, &index, &global, &schema, &natives, path).expect("decode texture"))
    }

    /// The body texture decodes to exactly the RGBA the research pipeline wrote to
    /// `tex-Body_C.png` (2048², BC1). The checksum ties the runtime port to that oracle.
    #[test]
    fn body_texture_matches_the_research_png() {
        let path = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Body_C.uasset";
        let Some(tex) = load_texture(path) else {
            return;
        };
        assert_eq!((tex.width, tex.height), (2048, 2048));
        assert_eq!(fnv1a64(&tex.rgba), 0x96e9_9dbb_ab1d_bcd8);
        assert_eq!(tex.pixel(0, 0), Some([0xac, 0x87, 0x7d, 0xff]));
    }

    /// The PartA texture is BC3 (DXT5) and decodes to the PartA reference PNG.
    #[test]
    fn parta_texture_matches_the_research_png() {
        let path = "/M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/001/TexturePC_004_U_CMN_001_PartA_C.uasset";
        let Some(tex) = load_texture(path) else {
            return;
        };
        assert_eq!((tex.width, tex.height), (2048, 2048));
        assert_eq!(fnv1a64(&tex.rgba), 0xced4_acad_7db3_c13c);
    }
}
