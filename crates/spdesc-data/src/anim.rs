//! Decoding ACL (Animation Compression Library) `compressed_tracks` payloads.
//!
//! The First Descendant cooks its `UAnimSequence` clips with the Unreal ACL plugin
//! (`AnimBoneCompressionCodec_ACL`), so a clip's bulk is an ACL 2.0.0 `compressed_tracks` blob. This
//! module is a from-scratch port of ACL 2.0.0's scalar decompression path (the reference is the MIT
//! `nfrechette/acl` library, version `v02_00_00`), validated against that library acting as an
//! offline oracle. It reads the blob's header, sub-track type bit sets, quantization metadata, and
//! range data, and reconstructs the per-frame `qvv` transforms.
//!
//! Only the pieces the shipped clips use are implemented: transform tracks, uniform sampling,
//! versions `v02_00_00`, with or without multiple segments, and the full/drop-w/full/variable
//! rotation and translation formats. Database streaming, metadata, and stripped keyframes are
//! rejected rather than guessed.

use anyhow::{Result, bail};

/// `buffer_tag32::compressed_tracks`.
const TAG_COMPRESSED_TRACKS: u32 = 0xac11_ac11;
/// The ACL binary version this decoder implements.
const VERSION_V0_02_00_00: u16 = 7;

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn f32le(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
/// Load a big-endian `u64` at `byte` (ACL's quantized data is big-endian).
fn be_u64(b: &[u8], byte: usize) -> u64 {
    u64::from_be_bytes([
        b[byte],
        b[byte + 1],
        b[byte + 2],
        b[byte + 3],
        b[byte + 4],
        b[byte + 5],
        b[byte + 6],
        b[byte + 7],
    ])
}

fn align_to(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

/// One frame's reconstructed transform for one track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackSample {
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

/// A decompressed clip: `num_samples` frames of `num_tracks` tracks each.
#[derive(Clone, Debug)]
pub struct AnimClip {
    pub num_tracks: u32,
    pub num_samples: u32,
    pub sample_rate: f32,
    /// Frame-major: index `sample * num_tracks + track`.
    pub samples: Vec<TrackSample>,
}

impl AnimClip {
    pub fn frame(&self, sample: usize) -> &[TrackSample] {
        let n = self.num_tracks as usize;
        &self.samples[sample * n..(sample + 1) * n]
    }
}

/// The static header of a parsed `compressed_tracks` blob.
struct Header {
    num_tracks: u32,
    num_samples: u32,
    sample_rate: f32,
    has_scale: bool,
    default_scale: f32,
    rotation_format: u8,
    translation_format: u8,
    scale_format: u8,
    // transform_tracks_header
    tth_off: usize,
    num_segments: u32,
    num_animated_variable_sub_tracks: u32,
    num_animated_rotation_sub_tracks: u32,
    num_animated_translation_sub_tracks: u32,
    num_constant_rotation_samples: u32,
    num_constant_translation_samples: u32,
    num_constant_scale_samples: u32,
    segment_headers_off: usize,
    sub_track_types_off: usize,
    constant_track_data_off: usize,
    clip_range_data_off: usize,
}

fn parse_header(blob: &[u8]) -> Result<Header> {
    if blob.len() < 40 {
        bail!("ACL blob too small ({} bytes)", blob.len());
    }
    let size = u32le(blob, 0);
    if size as usize != blob.len() {
        bail!("ACL size field {size} != blob length {}", blob.len());
    }
    let tag = u32le(blob, 8);
    if tag != TAG_COMPRESSED_TRACKS {
        bail!("not an ACL compressed_tracks buffer (tag {tag:#010x})");
    }
    let version = u16le(blob, 12);
    if version != VERSION_V0_02_00_00 {
        bail!("unsupported ACL version {version} (this decoder implements v02_00_00 = 7)");
    }
    let algorithm_type = blob[14];
    if algorithm_type != 0 {
        bail!("unsupported ACL algorithm type {algorithm_type} (only uniformly_sampled)");
    }
    let track_type = blob[15];
    if track_type != 12 {
        bail!("unsupported ACL track type {track_type} (only qvvf = 12)");
    }
    let num_tracks = u32le(blob, 16);
    let num_samples = u32le(blob, 20);
    let sample_rate = f32le(blob, 24);
    let misc = u32le(blob, 28);
    if misc & (1 << 31) != 0 {
        bail!("ACL blob has optional metadata, which this decoder does not read");
    }
    if misc & (1 << 8) != 0 {
        bail!("ACL blob is split into a compressed database, which this decoder does not read");
    }
    let tth_off = 32usize;
    if blob.len() < tth_off + 52 {
        bail!("ACL transform header overruns the blob");
    }
    Ok(Header {
        num_tracks,
        num_samples,
        sample_rate,
        has_scale: misc & 1 != 0,
        default_scale: f32::from(((misc >> 1) & 1) as u8),
        rotation_format: ((misc >> 4) & 15) as u8,
        translation_format: ((misc >> 3) & 1) as u8,
        scale_format: ((misc >> 2) & 1) as u8,
        tth_off,
        num_segments: u32le(blob, tth_off),
        num_animated_variable_sub_tracks: u32le(blob, tth_off + 4),
        num_animated_rotation_sub_tracks: u32le(blob, tth_off + 8),
        num_animated_translation_sub_tracks: u32le(blob, tth_off + 12),
        num_constant_rotation_samples: u32le(blob, tth_off + 20),
        num_constant_translation_samples: u32le(blob, tth_off + 24),
        num_constant_scale_samples: u32le(blob, tth_off + 28),
        segment_headers_off: u32le(blob, tth_off + 36) as usize,
        sub_track_types_off: u32le(blob, tth_off + 40) as usize,
        constant_track_data_off: u32le(blob, tth_off + 44) as usize,
        clip_range_data_off: u32le(blob, tth_off + 48) as usize,
    })
}

/// The `segment_header` fields (16 bytes).
#[derive(Clone, Copy)]
struct SegmentHeader {
    animated_pose_bit_size: u32,
    animated_rotation_bit_size: u32,
    animated_translation_bit_size: u32,
    segment_data_off: usize,
}

fn read_segment(blob: &[u8], header: &Header, index: usize) -> Result<SegmentHeader> {
    let at = header.tth_off + header.segment_headers_off + index * 16;
    if at + 16 > blob.len() {
        bail!("ACL segment {index} overruns the blob");
    }
    Ok(SegmentHeader {
        animated_pose_bit_size: u32le(blob, at),
        animated_rotation_bit_size: u32le(blob, at + 4),
        animated_translation_bit_size: u32le(blob, at + 8),
        segment_data_off: u32le(blob, at + 12) as usize,
    })
}

// ── quantization unpackers (ACL vector4_packing.h) ────────────────────────────

/// `unpack_vector3_uXX_unsafe`: `num_bits` per component, big-endian, in `[0, 1)`.
fn unpack_vec3_uxx(num_bits: u32, data: &[u8], mut bit: usize) -> [f32; 3] {
    let mask = (1u32 << num_bits) - 1;
    let inv = if num_bits == 0 {
        1.0
    } else {
        1.0 / mask as f32
    };
    let mut out = [0.0f32; 3];
    for v in &mut out {
        let byte = bit / 8;
        let shift = 32 - num_bits - (bit % 8) as u32;
        let raw = u32le(data, byte);
        let raw = raw.swap_bytes();
        *v = ((raw >> shift) & mask) as f32 * inv;
        bit += num_bits as usize;
    }
    out
}

/// `unpack_vector3_96_unsafe(data, bit_offset)`: three big-endian `f32` at a bit offset.
fn unpack_vec3_96(data: &[u8], bit: usize) -> [f32; 3] {
    let byte = bit / 8;
    let shift = (bit % 8) as u64;
    let mut out = [0.0f32; 3];
    for (i, v) in out.iter_mut().enumerate() {
        let v64 = be_u64(data, byte + i * 4) << shift;
        *v = f32::from_bits((v64 >> 32) as u32);
    }
    out
}

/// `unpack_vector4_128_unsafe(data, bit_offset)`: four big-endian `f32` at a bit offset.
fn unpack_vec4_128(data: &[u8], bit: usize) -> [f32; 4] {
    let byte = bit / 8;
    let shift = (bit % 8) as u64;
    let mut out = [0.0f32; 4];
    for (i, v) in out.iter_mut().enumerate() {
        let v64 = be_u64(data, byte + i * 4) << shift;
        *v = f32::from_bits((v64 >> 32) as u32);
    }
    out
}

/// `unpack_vector3_u48_unsafe`: three little-endian `u16` normalised to `[0, 1]`.
fn unpack_vec3_u48(data: &[u8]) -> [f32; 3] {
    [
        f32::from(u16le(data, 0)) / 65535.0,
        f32::from(u16le(data, 2)) / 65535.0,
        f32::from(u16le(data, 4)) / 65535.0,
    ]
}

/// `unpack_vector3_u24_unsafe`: three `u8` normalised to `[0, 1]`.
fn unpack_vec3_u24(data: &[u8]) -> [f32; 3] {
    [
        f32::from(data[0]) / 255.0,
        f32::from(data[1]) / 255.0,
        f32::from(data[2]) / 255.0,
    ]
}

/// `quat_from_positive_w`: rebuild `w` for a quaternion whose w component was dropped.
fn quat_from_positive_w(xyz: [f32; 3]) -> [f32; 4] {
    let w_sq = 1.0 - xyz[0] * xyz[0] - xyz[1] * xyz[1] - xyz[2] * xyz[2];
    [xyz[0], xyz[1], xyz[2], w_sq.abs().sqrt()]
}

fn quat_full(data: &[u8], byte: usize) -> [f32; 4] {
    [
        f32le(data, byte),
        f32le(data, byte + 4),
        f32le(data, byte + 8),
        f32le(data, byte + 12),
    ]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

// ── sub-track types ───────────────────────────────────────────────────────────

const KIND_DEFAULT: u8 = 0;
const KIND_CONSTANT: u8 = 1;
#[allow(dead_code)]
const KIND_ANIMATED: u8 = 2;

/// The 2-bit kind of sub-track `pos` within a packed entry (sub-track 0 in the top bits).
fn kind(entry: u32, pos: usize) -> u8 {
    ((entry >> (30 - 2 * pos)) & 3) as u8
}

/// Per-segment animated sampling pointers, advanced as groups of four sub-tracks are read.
struct SegmentData<'a> {
    format: &'a [u8],
    segment_range: &'a [u8],
    animated: &'a [u8],
    bit_offset: usize,
}

/// Everything needed to decode one frame from one segment.
struct FrameDecoder<'a> {
    header: &'a Header,
    /// Rotation / translation / scale format & range pointers (not yet offset by group).
    rot: SegmentData<'a>,
    trans: SegmentData<'a>,
    scale: SegmentData<'a>,
    clip_range: &'a [u8],
    has_segments: bool,
}

/// Find the embedded `compressed_tracks` buffer inside a cooked `UAnimSequence` export.
///
/// The sequence's native tail carries the ACL blob inline, behind a codec handle this parser does
/// not need to read. A `compressed_tracks` buffer is self-describing and strongly tagged: it begins
/// with its own size, then the `compressed_tracks` tag and version. The search accepts the first
/// such buffer whose header validates, which ties the slice to the format rather than to an
/// assumed tail layout.
pub fn find_blob(export: &[u8]) -> Option<&[u8]> {
    let needle = TAG_COMPRESSED_TRACKS.to_le_bytes();
    for i in 8..export.len().saturating_sub(4) {
        if export[i..i + 4] != needle {
            continue;
        }
        let start = i - 8;
        let size = u32le(export, start) as usize;
        if size < 64 || start + size > export.len() {
            continue;
        }
        if u16le(export, i + 4) == VERSION_V0_02_00_00 {
            return Some(&export[start..start + size]);
        }
    }
    None
}

fn build_frame_decoder<'a>(
    blob: &'a [u8],
    header: &'a Header,
    segment_index: usize,
    segment_key_frame: u32,
) -> Result<FrameDecoder<'a>> {
    let tth = header.tth_off;
    let seg = read_segment(blob, header, segment_index)?;
    let n_var = header.num_animated_variable_sub_tracks as usize;

    // `get_segment_data`: format-per-track, then (multi-segment) range data, then the animated data,
    // each aligned as ACL wrote them.
    let format_at = tth + seg.segment_data_off;
    let range_at = tth + align_to(seg.segment_data_off + n_var, 2);
    let range_size = if header.num_segments > 1 {
        6 * n_var
    } else {
        0
    };
    let animated_at = tth + align_to(range_at - tth + range_size, 4);
    if blob.len() < animated_at.saturating_add(8) {
        bail!("ACL segment {segment_index} data is out of bounds");
    }

    let rot_count_padded = align_to(header.num_animated_rotation_sub_tracks as usize, 4);
    let trans_count = header.num_animated_translation_sub_tracks as usize;
    // A missing range section is represented by an empty slice.
    let range = |off: usize| -> &[u8] {
        if header.num_segments > 1 {
            &blob[range_at + off..]
        } else {
            &blob[range_at..range_at]
        }
    };

    let clip_range = &blob[tth + header.clip_range_data_off..];
    let base_bit = segment_key_frame as usize * seg.animated_pose_bit_size as usize;

    Ok(FrameDecoder {
        header,
        rot: SegmentData {
            format: &blob[format_at..],
            segment_range: range(0),
            animated: &blob[animated_at..],
            bit_offset: base_bit,
        },
        trans: SegmentData {
            format: &blob[format_at + rot_count_padded..],
            segment_range: range(rot_count_padded * 6),
            animated: &blob[animated_at..],
            bit_offset: base_bit + seg.animated_rotation_bit_size as usize,
        },
        scale: SegmentData {
            format: &blob[format_at + rot_count_padded + trans_count..],
            segment_range: range(rot_count_padded * 6 + trans_count * 6),
            animated: &blob[animated_at..],
            bit_offset: base_bit
                + seg.animated_rotation_bit_size as usize
                + seg.animated_translation_bit_size as usize,
        },
        clip_range,
        has_segments: header.num_segments > 1,
    })
}

/// Decode one animated rotation sub-track: `index` is its ordinal among the animated rotation
/// sub-tracks (the data groups them four at a time).
fn animated_rotation(dec: &FrameDecoder, index: usize) -> [f32; 4] {
    let variable = dec.header.rotation_format == 3;
    let full = dec.header.rotation_format == 0;
    let group = index / 4;
    let within = index % 4;
    let count = dec.header.num_animated_rotation_sub_tracks as usize;
    let group_size = (count - group * 4).min(4);
    // The animated sample data is contiguous, so its bit offset is the sum of the preceding
    // samples' widths; the per-track format metadata is padded to groups of four.
    let skip: u32 = dec.rot.format[..index].iter().map(|b| u32::from(*b)).sum();
    let bit = dec.rot.bit_offset + skip as usize * 3;
    let mut segment_ignore = 0u32;
    let mut clip_ignore = 0u32;

    let mut rotation_as_vec;
    if variable {
        let num_bits = u32::from(dec.rot.format[index]);
        if num_bits == 0 {
            let srd = &dec.rot.segment_range[group * 24 + within..group * 24 + within + 21];
            let x = (u32::from(srd[0]) << 8) | u32::from(srd[4]);
            let y = (u32::from(srd[8]) << 8) | u32::from(srd[12]);
            let z = (u32::from(srd[16]) << 8) | u32::from(srd[20]);
            rotation_as_vec = [x as f32 / 65535.0, y as f32 / 65535.0, z as f32 / 65535.0];
            segment_ignore = 0xFF;
        } else if num_bits == 32 {
            rotation_as_vec = unpack_vec3_96(dec.rot.animated, bit);
            segment_ignore = 0xFF;
            clip_ignore = 0xFF;
        } else {
            rotation_as_vec = unpack_vec3_uxx(num_bits, dec.rot.animated, bit);
        }
    } else if full {
        return unpack_vec4_128(dec.rot.animated, dec.rot.bit_offset + index * 128);
    } else {
        rotation_as_vec = unpack_vec3_96(dec.rot.animated, dec.rot.bit_offset + index * 96);
        segment_ignore = 0xFF;
        clip_ignore = 0xFF;
    }

    if variable {
        if dec.has_segments && segment_ignore == 0 {
            let srd = &dec.rot.segment_range[group * 24 + within..group * 24 + within + 21];
            let min = [
                f32::from(srd[0]) / 255.0,
                f32::from(srd[4]) / 255.0,
                f32::from(srd[8]) / 255.0,
            ];
            let ext = [
                f32::from(srd[12]) / 255.0,
                f32::from(srd[16]) / 255.0,
                f32::from(srd[20]) / 255.0,
            ];
            rotation_as_vec = add3(mul3(rotation_as_vec, ext), min);
        }
        if clip_ignore == 0 {
            // Rotation clip range is SOA: six runs of `group_size` floats.
            let start = group * 4 * 24;
            let base = &dec.clip_range[start..start + group_size * 6 * 4];
            let get = |run: usize, i: usize| f32le(base, (run * group_size + i) * 4);
            let min = [get(0, within), get(1, within), get(2, within)];
            let ext = [get(3, within), get(4, within), get(5, within)];
            rotation_as_vec = add3(mul3(rotation_as_vec, ext), min);
        }
    }
    quat_from_positive_w(rotation_as_vec)
}

/// Decode one animated translation/scale sub-track: `index` is its ordinal among the animated
/// sub-tracks of that type (the data is contiguous, one entry per sub-track).
fn animated_vector3(
    dec: &FrameDecoder,
    seg: &SegmentData,
    clip_range: &[u8],
    variable: bool,
    index: usize,
) -> [f32; 3] {
    if !variable {
        return unpack_vec3_96(seg.animated, seg.bit_offset + index * 96);
    }
    let skip: u32 = seg.format[..index].iter().map(|b| u32::from(*b)).sum();
    let bit = seg.bit_offset + skip as usize * 3;
    let num_bits = u32::from(seg.format[index]);
    let mut ignore = 0u32;
    let sample;
    if num_bits == 0 {
        let segment_range = &seg.segment_range[index * 6..index * 6 + 6];
        sample = unpack_vec3_u48(segment_range);
        ignore = 0x01;
    } else if num_bits == 32 {
        sample = unpack_vec3_96(seg.animated, bit);
        ignore = 0x03;
    } else {
        sample = unpack_vec3_uxx(num_bits, seg.animated, bit);
    }
    let mut sample = sample;
    if dec.has_segments && (ignore & 0x01) == 0 {
        let segment_range = &seg.segment_range[index * 6..index * 6 + 6];
        let min = unpack_vec3_u24(segment_range);
        let ext = unpack_vec3_u24(&segment_range[3..]);
        sample = add3(mul3(sample, ext), min);
    }
    if (ignore & 0x02) == 0 {
        let clip_range = &clip_range[index * 24..index * 24 + 24];
        let min = [
            f32le(clip_range, 0),
            f32le(clip_range, 4),
            f32le(clip_range, 8),
        ];
        let ext = [
            f32le(clip_range, 12),
            f32le(clip_range, 16),
            f32le(clip_range, 20),
        ];
        sample = add3(mul3(sample, ext), min);
    }
    sample
}

/// Unpack the constant rotation samples (stored `drop_w` in SOA groups of four, or full AOS).
fn constant_rotations(blob: &[u8], header: &Header) -> Result<Vec<[f32; 4]>> {
    let count = header.num_constant_rotation_samples as usize;
    let base = header.tth_off + header.constant_track_data_off;
    let variable_or_drop_w = header.rotation_format != 0;
    let mut out = Vec::with_capacity(count);
    if !variable_or_drop_w {
        // Full precision AOS, 16 bytes per sample.
        for i in 0..count {
            out.push(quat_full(blob, base + i * 16));
        }
        return Ok(out);
    }
    // drop-w in SOA groups of up to four: xxxx, yyyy, zzzz floats.
    let mut group_start = base;
    let mut i = 0;
    while i < count {
        let group_size = (count - i).min(4);
        let load = group_size * 4;
        let x = group_start;
        let y = group_start + load;
        let z = group_start + load * 2;
        for j in 0..group_size {
            let xyz = [
                f32le(blob, x + j * 4),
                f32le(blob, y + j * 4),
                f32le(blob, z + j * 4),
            ];
            out.push(quat_from_positive_w(xyz));
        }
        group_start += load * 3;
        i += group_size;
    }
    Ok(out)
}

/// Unpack the constant translation or scale samples (AOS `float3`, 12 bytes each).
fn constant_vectors(blob: &[u8], base: usize, count: usize) -> Vec<[f32; 3]> {
    (0..count)
        .map(|i| {
            [
                f32le(blob, base + i * 12),
                f32le(blob, base + i * 12 + 4),
                f32le(blob, base + i * 12 + 8),
            ]
        })
        .collect()
}

/// Find the segment and segment-relative key frame for a global key frame.
fn locate_segment(blob: &[u8], header: &Header, key_frame: u32) -> Result<(usize, u32)> {
    if header.num_segments <= 1 {
        return Ok((0, key_frame));
    }
    let start_at = header.tth_off + align_to(52, 4);
    let approx = header.num_samples / header.num_segments;
    let approx_segment = key_frame.checked_div(approx).unwrap_or(0);
    let start = approx_segment.saturating_sub(1);
    let end = start + 4;
    let index = |i: usize| -> u32 { u32le(blob, start_at + i * 4) };
    let mut seg0 = 0usize;
    for segment_index in start..end {
        if key_frame < index(segment_index as usize) {
            seg0 = segment_index as usize - 1;
            break;
        }
    }
    Ok((seg0, key_frame - index(seg0)))
}

/// Decode all frames of an ACL `compressed_tracks` blob (nearest sampling).
pub fn decode(blob: &[u8]) -> Result<AnimClip> {
    let header = parse_header(blob)?;
    let num_tracks = header.num_tracks as usize;
    let num_samples = header.num_samples as usize;
    if num_tracks == 0 || num_samples == 0 {
        bail!("ACL blob is empty");
    }

    let constants_rot = constant_rotations(blob, &header)?;
    let constant_tb = header.tth_off + header.constant_track_data_off;
    let packed_rotation_size = if header.rotation_format == 0 { 16 } else { 12 };
    let constant_trans = constant_vectors(
        blob,
        constant_tb + packed_rotation_size * header.num_constant_rotation_samples as usize,
        header.num_constant_translation_samples as usize,
    );
    let constant_scale = constant_vectors(
        blob,
        constant_tb
            + packed_rotation_size * header.num_constant_rotation_samples as usize
            + 12 * header.num_constant_translation_samples as usize,
        header.num_constant_scale_samples as usize,
    );

    // Sub-track type bit sets: rotation, translation, scale sections of `num_entries` u32s each.
    let entries = num_tracks.div_ceil(16);
    let types_at = header.tth_off + header.sub_track_types_off;
    let section = |s: usize| -> Vec<u32> {
        (0..entries)
            .map(|e| u32le(blob, types_at + (s * entries + e) * 4))
            .collect()
    };
    let rot_types = section(0);
    let trans_types = section(1);
    let scale_types = section(2);

    let mut out = vec![
        TrackSample {
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation: [0.0; 3],
            scale: [header.default_scale; 3],
        };
        num_tracks * num_samples
    ];

    for sample in 0..num_samples {
        let sample_index = sample as f32;
        let key_frame0 = sample_index as u32;
        let key_frame1 = (key_frame0 + 1).min(header.num_samples - 1);
        let alpha = (sample_index - key_frame0 as f32 + 0.5).floor();
        let (key_frame, chosen) = if alpha <= 0.0 {
            (key_frame0, 0)
        } else {
            (key_frame1, 1)
        };
        let (seg_index, seg_key_frame) = locate_segment(blob, &header, key_frame)?;
        let dec = build_frame_decoder(blob, &header, seg_index, seg_key_frame)?;
        let _ = chosen;

        // Defaults + constants, then animated, per type. Constant consumption follows bit-set order.
        let mut cr = 0usize;
        let mut ct = 0usize;
        let mut cs = 0usize;
        let mut anim_rot = 0usize;
        let mut anim_trans = 0usize;
        let mut anim_scale = 0usize;

        let frame = &mut out[sample * num_tracks..(sample + 1) * num_tracks];

        decode_type(&rot_types, entries, num_tracks, |track, k| match k {
            KIND_DEFAULT => frame[track].rotation = [0.0, 0.0, 0.0, 1.0],
            KIND_CONSTANT => {
                frame[track].rotation = constants_rot[cr];
                cr += 1;
            }
            _ => {
                frame[track].rotation = animated_rotation(&dec, anim_rot);
                anim_rot += 1;
            }
        });
        decode_type(&trans_types, entries, num_tracks, |track, k| match k {
            KIND_DEFAULT => frame[track].translation = [0.0; 3],
            KIND_CONSTANT => {
                frame[track].translation = constant_trans[ct];
                ct += 1;
            }
            _ => {
                let rot_clip = header.num_animated_rotation_sub_tracks as usize * 24;
                let trans_clip = &dec.clip_range[rot_clip..];
                frame[track].translation = animated_vector3(
                    &dec,
                    &dec.trans,
                    trans_clip,
                    header.translation_format == 1,
                    anim_trans,
                );
                anim_trans += 1;
            }
        });
        if header.has_scale {
            decode_type(&scale_types, entries, num_tracks, |track, k| match k {
                KIND_DEFAULT => frame[track].scale = [header.default_scale; 3],
                KIND_CONSTANT => {
                    frame[track].scale = constant_scale[cs];
                    cs += 1;
                }
                _ => {
                    let scale_clip = (header.num_animated_rotation_sub_tracks
                        + header.num_animated_translation_sub_tracks)
                        as usize
                        * 24;
                    let scale_clip = &dec.clip_range[scale_clip..];
                    frame[track].scale = animated_vector3(
                        &dec,
                        &dec.scale,
                        scale_clip,
                        header.scale_format == 1,
                        anim_scale,
                    );
                    anim_scale += 1;
                }
            });
        }
    }

    Ok(AnimClip {
        num_tracks: header.num_tracks,
        num_samples: header.num_samples,
        sample_rate: header.sample_rate,
        samples: out,
    })
}

/// Walk the sub-track bit set, calling `visit(track_index, kind)` for each of the first
/// `num_tracks` positions, in the order ACL's decoder visits them.
fn decode_type<F>(types: &[u32], entries: usize, num_tracks: usize, mut visit: F)
where
    F: FnMut(usize, u8),
{
    for (entry_index, entry) in types.iter().enumerate().take(entries) {
        for pos in 0..16 {
            let track = entry_index * 16 + pos;
            if track >= num_tracks {
                break;
            }
            visit(track, kind(*entry, pos));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_non_acl_buffer() {
        assert!(decode(&[0u8; 64]).is_err());
    }

    #[test]
    fn quantization_helpers_behave() {
        // A one-bit field: the top bit normalises to 1, the rest to 0.
        let data = [0x80u8, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(unpack_vec3_uxx(1, &data, 0), [1.0, 0.0, 0.0]);
        // u48/u24 are the unsigned maxima.
        assert_eq!(
            unpack_vec3_u48(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]),
            [1.0; 3]
        );
        assert_eq!(unpack_vec3_u24(&[0xff, 0xff, 0xff]), [1.0; 3]);
        // Rebuilding w from the origin gives the identity quaternion.
        assert_eq!(quat_from_positive_w([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn sub_track_kind_reads_the_top_bits_first() {
        // sub-track 0 in bits 31..30, sub-track 1 in 29..28.
        assert_eq!(kind(0b10_01_00_00 << 24, 0), KIND_ANIMATED);
        assert_eq!(kind(0b10_01_00_00 << 24, 1), KIND_CONSTANT);
        assert_eq!(kind(0b10_01_00_00 << 24, 2), KIND_DEFAULT);
    }

    /// Decode the Ultimate Bunny idle clip from this machine's folder and compare a spread of
    /// frames to values the ACL library produced when it decompressed the same blob (the offline
    /// oracle). Ties the Rust port to the reference codec's output.
    #[test]
    fn idle_clip_matches_the_acl_oracle() {
        const CLIP: &str = "/M1/Content/Characters/PC/Anim/F/CMN/Sequence/IdleF_0LT_Idle.uasset";
        // (track, sample) -> rotation xyzw, translation xyz, scale xyz, from ACL v2.0.0.
        const EXPECTED: &[(usize, usize, [f32; 10])] = &[
            (0, 0, [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
            (
                1,
                0,
                [
                    0.0288, 0.0214, 0.8880, 0.4584, 0.0, 0.0, 101.4, 1.0, 1.0, 1.0,
                ],
            ),
            (
                1,
                220,
                [
                    0.0288, 0.0214, 0.8880, 0.4584, 0.0, 0.0, 101.4, 1.0, 1.0, 1.0,
                ],
            ),
            (
                5,
                110,
                [
                    0.0016, 0.0136, 0.0039, 0.9999, 8.9898, 0.0068, 0.0, 1.0, 1.0, 1.0,
                ],
            ),
            (
                50,
                110,
                [0.0, 0.0, 0.0, 1.0, -0.0001, 0.0, -7.0282, 1.0, 1.0, 1.0],
            ),
            (100, 220, [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
            (
                305,
                110,
                [
                    -0.5454, 0.4812, 0.5333, 0.4318, -11.7579, 12.6602, -89.1817, 1.0, 1.0, 1.0,
                ],
            ),
        ];
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
        let index = store.directory_index().expect("directory index");
        let chunk = *index.get(CLIP).expect("idle clip is indexed");
        let bytes = store.read_chunk(chunk as usize).expect("read clip chunk");
        let global = crate::zen::GlobalNames::read(&paks).expect("global.ucas");
        let package = crate::zen::ZenPackage::parse(&bytes, &global).expect("parse package");
        let i = package
            .exports
            .iter()
            .position(|e| e.class.contains("AnimSequence"))
            .expect("an AnimSequence export");
        let blob = package
            .export_data_variants(&bytes, i)
            .into_iter()
            .find_map(find_blob)
            .expect("an embedded ACL blob");
        let clip = decode(blob).expect("decode the idle clip");
        assert_eq!(clip.num_tracks, 306);
        assert_eq!(clip.num_samples, 221);
        assert_eq!(clip.sample_rate, 30.0);
        for &(track, sample, want) in EXPECTED {
            let got = clip.samples[sample * clip.num_tracks as usize + track];
            let flat = [
                got.rotation[0],
                got.rotation[1],
                got.rotation[2],
                got.rotation[3],
                got.translation[0],
                got.translation[1],
                got.translation[2],
                got.scale[0],
                got.scale[1],
                got.scale[2],
            ];
            for (c, (g, w)) in flat.iter().zip(want.iter()).enumerate() {
                assert!(
                    (g - w).abs() < 1e-3,
                    "track {track} sample {sample} component {c}: got {g}, oracle {w}"
                );
            }
        }
    }
}
