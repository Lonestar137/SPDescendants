//! Decoding Modern Warfare 2's `.iwi` images.
//!
//! The interface art ships only this way: `<MW2_DATA>/main/iw_*.iwd` holds `.iwi` files, and the
//! rank and prestige icons are among them (`images/cardicon_rank_*.iwi`,
//! `images/cardicon_prestige_*.iwi` — 64x64 DXT5, one mip). The header layout is `asset_iw4`'s
//! (vendored from IW4L); the block decoding is here, so the app gets plain RGBA it can hand to
//! Bevy.
//!
//! Only the largest mip is decoded, which is the one at the front of the file, and only the
//! formats the art uses: the plain bitmaps and DXT1/DXT3/DXT5. A wavelet image is refused rather
//! than guessed at.

use asset_iw4::img_format::{ImgFormatKind, img_format_info};
use asset_iw4::iwi::{IWI_V8_HEADER_LEN, IwiHeader, IwiHeaderError};

/// A decoded image: tightly packed RGBA8 rows, top-left first.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Image")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

/// Why an `.iwi` could not be decoded.
#[derive(Debug)]
pub enum IwiError {
    /// The 32-byte header is not an `.iwi` header.
    Header(IwiHeaderError),
    /// The format code is not one this decoder knows.
    UnsupportedFormat(u8),
    /// A wavelet image, which needs the wavelet decoder.
    Wavelet(u8),
    /// A cubemap or texture array, which the interface art never is.
    Layered(u16),
    /// Zero width or height.
    Empty,
    /// Fewer bytes than the image needs.
    Truncated { needed: usize, got: usize },
}

impl std::fmt::Display for IwiError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Header(e) => write!(out, "not an .iwi image: {e:?}"),
            Self::UnsupportedFormat(format) => write!(out, "unsupported .iwi format {format}"),
            Self::Wavelet(format) => {
                write!(
                    out,
                    ".iwi format {format} is wavelet-compressed, not decoded here"
                )
            }
            Self::Layered(depth) => write!(out, "an .iwi with {depth} layers"),
            Self::Empty => write!(out, "an .iwi with no pixels"),
            Self::Truncated { needed, got } => {
                write!(out, "the .iwi needs {needed} bytes, {got} present")
            }
        }
    }
}

impl std::error::Error for IwiError {}

impl From<IwiHeaderError> for IwiError {
    fn from(e: IwiHeaderError) -> Self {
        Self::Header(e)
    }
}

/// Decode the largest mip of an `.iwi`.
pub fn decode(bytes: &[u8]) -> Result<Image, IwiError> {
    let header = IwiHeader::parse(bytes)?;
    decode_with(&header, bytes.get(IWI_V8_HEADER_LEN..).unwrap_or(&[]))
}

/// [`decode`] over the pixel data alone, for a caller that already parsed the header.
pub fn decode_with(header: &IwiHeader, pixels: &[u8]) -> Result<Image, IwiError> {
    let width = u32::from(header.width);
    let height = u32::from(header.height);
    if width == 0 || height == 0 {
        return Err(IwiError::Empty);
    }
    if header.depth > 1 {
        return Err(IwiError::Layered(header.depth));
    }
    let info = img_format_info(header.format).ok_or(IwiError::UnsupportedFormat(header.format))?;

    let rgba = match info.kind {
        ImgFormatKind::Wavelet => return Err(IwiError::Wavelet(header.format)),
        ImgFormatKind::PlainBitmap => {
            let needed = width as usize * height as usize * info.channels as usize;
            let source = pixels.get(..needed).ok_or(IwiError::Truncated {
                needed,
                got: pixels.len(),
            })?;
            plain_to_rgba(source, info.channels)
        }
        ImgFormatKind::Dxt => {
            // 11, 12 and 13 are DXT1, DXT3 and DXT5, in the order the engine names them.
            let block = match header.format {
                11 => Block::Dxt1,
                12 => Block::Dxt3,
                13 => Block::Dxt5,
                other => return Err(IwiError::UnsupportedFormat(other)),
            };
            let needed = block.bytes(width, height);
            let source = pixels.get(..needed).ok_or(IwiError::Truncated {
                needed,
                got: pixels.len(),
            })?;
            block.decode(source, width, height)
        }
    };
    Ok(Image {
        width,
        height,
        rgba,
    })
}

/// A plain bitmap's channels expanded to RGBA8. IW4's plain formats store blue first.
fn plain_to_rgba(source: &[u8], channels: u8) -> Vec<u8> {
    let pixels = source.len() / channels as usize;
    let mut out = Vec::with_capacity(pixels * 4);
    for i in 0..pixels {
        let p = &source[i * channels as usize..];
        let (r, g, b, a) = match channels {
            1 => (p[0], p[0], p[0], 0xff),
            2 => (p[1], p[1], p[1], p[0]),
            3 => (p[2], p[1], p[0], 0xff),
            _ => (p[2], p[1], p[0], p[3]),
        };
        out.extend_from_slice(&[r, g, b, a]);
    }
    out
}

/// The three block-compressed formats the art uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Block {
    /// BC1: 8 bytes per 4x4 block, colour and a 1-bit alpha mode.
    Dxt1,
    /// BC2: 16 bytes, explicit 4-bit alpha then colour.
    Dxt3,
    /// BC3: 16 bytes, interpolated alpha then colour.
    Dxt5,
}

impl Block {
    fn bytes(self, width: u32, height: u32) -> usize {
        let blocks = width.div_ceil(4) as usize * height.div_ceil(4) as usize;
        blocks * if self == Block::Dxt1 { 8 } else { 16 }
    }

    /// Decode to RGBA8, one 4x4 block at a time.
    fn decode(self, source: &[u8], width: u32, height: u32) -> Vec<u8> {
        let stride = width as usize;
        let mut out = vec![0u8; stride * height as usize * 4];
        let mut at = 0usize;
        for by in (0..height).step_by(4) {
            for bx in (0..width).step_by(4) {
                let (alpha, colour_at) = match self {
                    Block::Dxt1 => ([0xffu8; 16], at),
                    Block::Dxt3 => (alpha_dxt3(&source[at..at + 8]), at + 8),
                    Block::Dxt5 => (alpha_dxt5(&source[at..at + 8]), at + 8),
                };
                let block = self.colour(&source[colour_at..colour_at + 8]);
                at += if self == Block::Dxt1 { 8 } else { 16 };
                for y in 0..4usize {
                    let py = by as usize + y;
                    if py >= height as usize {
                        break;
                    }
                    for x in 0..4usize {
                        let px = bx as usize + x;
                        if px >= width as usize {
                            break;
                        }
                        let texel = y * 4 + x;
                        let rgba = &block[texel];
                        let o = (py * stride + px) * 4;
                        out[o] = rgba[0];
                        out[o + 1] = rgba[1];
                        out[o + 2] = rgba[2];
                        out[o + 3] = if self == Block::Dxt1 {
                            rgba[3]
                        } else {
                            alpha[texel]
                        };
                    }
                }
            }
        }
        out
    }

    /// The 16 texels of a colour block, with DXT1's one-bit alpha applied.
    fn colour(self, colour: &[u8]) -> [[u8; 4]; 16] {
        let c0 = u16::from_le_bytes([colour[0], colour[1]]);
        let c1 = u16::from_le_bytes([colour[2], colour[3]]);
        let bits = u32::from_le_bytes([colour[4], colour[5], colour[6], colour[7]]);
        let e0 = rgb565(c0);
        let e1 = rgb565(c1);
        // DXT1 is four-colour only when its endpoints are ordered; otherwise the fourth entry is
        // transparent black. DXT3 and DXT5 always carry their alpha separately, so always four.
        let four_colour = self != Block::Dxt1 || c0 > c1;
        let mut palette = [[0u8; 4]; 4];
        palette[0] = [e0[0], e0[1], e0[2], 0xff];
        palette[1] = [e1[0], e1[1], e1[2], 0xff];
        if four_colour {
            for i in 0..3 {
                palette[2][i] = ((2 * u16::from(e0[i]) + u16::from(e1[i])) / 3) as u8;
                palette[3][i] = ((u16::from(e0[i]) + 2 * u16::from(e1[i])) / 3) as u8;
            }
            palette[2][3] = 0xff;
            palette[3][3] = 0xff;
        } else {
            for i in 0..3 {
                palette[2][i] = ((u16::from(e0[i]) + u16::from(e1[i])) / 2) as u8;
            }
            palette[2][3] = 0xff;
        }
        let mut out = [[0u8; 4]; 16];
        for (i, texel) in out.iter_mut().enumerate() {
            *texel = palette[((bits >> (2 * i)) & 3) as usize];
        }
        out
    }
}

/// DXT3: four bits of alpha per texel, straight.
fn alpha_dxt3(block: &[u8]) -> [u8; 16] {
    let bits = u64::from_le_bytes(block[..8].try_into().unwrap_or([0; 8]));
    let mut out = [0u8; 16];
    for (i, a) in out.iter_mut().enumerate() {
        *a = (((bits >> (4 * i)) & 0xf) * 17) as u8;
    }
    out
}

/// DXT5: two endpoint alphas and a 3-bit index per texel.
fn alpha_dxt5(block: &[u8]) -> [u8; 16] {
    let a0 = block[0];
    let a1 = block[1];
    let mut table = [0u8; 8];
    table[0] = a0;
    table[1] = a1;
    if a0 > a1 {
        for i in 1..7u16 {
            table[1 + i as usize] = ((a0 as u16 * (7 - i) + a1 as u16 * i) / 7) as u8;
        }
    } else {
        for i in 1..5u16 {
            table[1 + i as usize] = ((a0 as u16 * (5 - i) + a1 as u16 * i) / 5) as u8;
        }
        table[6] = 0;
        table[7] = 0xff;
    }
    let mut bits = 0u64;
    for (i, byte) in block[2..8].iter().enumerate() {
        bits |= u64::from(*byte) << (8 * i);
    }
    let mut out = [0u8; 16];
    for (i, a) in out.iter_mut().enumerate() {
        *a = table[((bits >> (3 * i)) & 7) as usize];
    }
    out
}

/// An RGB565 endpoint as 8-bit channels.
fn rgb565(value: u16) -> [u8; 3] {
    let r = (value >> 11) & 0x1f;
    let g = (value >> 5) & 0x3f;
    let b = value & 0x1f;
    [
        ((r as u32 * 255 + 15) / 31) as u8,
        ((g as u32 * 255 + 31) / 63) as u8,
        ((b as u32 * 255 + 15) / 31) as u8,
    ]
}
