# Texture format evidence

**Status:** validated for the supported virtual-texture path.  This note
describes the project reader's contract, not a general Unreal texture
specification.

## Scope and provenance

`crates/spdesc-data/src/texture.rs` reads a cooked `UTexture2D` into mip-0
RGBA. It was established from read-only observations of these `GAME_DATA`
inputs and is covered by data-gated tests in that module:

- `M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Body_C.uasset`
- `M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/001/TexturePC_004_U_CMN_001_PartA_C.uasset`
- `M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/Material/002/TexturePC_004_U_CMN_002_Helmet_N.uasset`
  (the BC5 normal-map regression input).

The reader locates the `.uasset` and its sibling `.ubulk` through the IoStore
directory index. Neither input, its decoded pixels, nor an extracted image is
committed. The property schema used to read the unversioned property block is a
machine-local mapping; see `crates/spdesc-data/src/mapping.rs` and
[../METHOD.md](../METHOD.md).

## Validated export framing

After the `Texture2D` property block, the supported export has:

1. one zero `u32` preamble value;
2. two strip-data flag pairs;
3. cooked and `bSerializeMipData` flags;
4. one or more pixel-format/platform-data records, terminated by `FName(None)`.

Each platform-data record carries a relative skip target. The reader requires
that parsing finishes exactly at that target and that the final terminator lands
at the end of the export. This detects a layout or schema mismatch before a
pixel is decoded.

The supported `FTexturePlatformData` form has no derived-data payload and no
CPU-copy payload. It contains dimensions, packed flags, a format name, optional
tail metadata, serialized mip descriptors, and a virtual-texture block. The
classic non-virtual mip path is intentionally unsupported.

## Validated virtual-texture rules

The tested inputs store their pixels in `FVirtualTextureBuiltData`, not in the
classic mip array. The reader relies on these rules:

- The virtual texture is cooked and has a bounded number of layers and chunks.
- Per-mip chunk indices and base offsets select the bulk-data region for a mip.
- Declared chunk byte lengths concatenate, in entry order, to exactly the
  sibling `.ubulk` length.
- A physical tile is `tile_size + 2 * tile_border` pixels wide and high. The
  decoded output copies only the central `tile_size × tile_size` region.
- Tile addresses use Morton order: X contributes the even bits and Y the odd
  bits, least-significant bit first.
- The declared layer tile-byte size must match the physical tile dimensions and
  the block format's byte density.
- Mip-0 tile-grid dimensions must agree with the platform dimensions.

Those checks are part of the implementation so malformed or unfamiliar inputs
fail explicitly rather than producing a plausibly scrambled image.

## Pixel-format coverage

The reader has these decode paths to RGBA8:

| Layer type | Evidence and decoder behavior |
| --- | --- |
| `PF_DXT1` | **Validated** BC1 colour decode |
| `PF_DXT5` | **Validated** BC3 colour/alpha decode |
| `PF_BC5` | **Validated** two-channel normal-map decode with reconstructed Z |
| `PF_BC4` | Implemented one-channel decode; no separate data-gated asset validation yet |
| `PF_DXT3` | Routes through the current BC3-style path; requires independent validation before reliance |

`PF_BC7` has a size model so its layout can be checked, but it has no pixel
decoder and is rejected. Derived-data payloads, CPU-copy payloads, non-virtual
textures, unfamiliar layer layouts, and malformed tile tables are likewise
unsupported until independently validated.

## Regression evidence

`body_texture_matches_the_research_png`,
`parta_texture_matches_the_research_png`, and
`helmet_normal_map_decodes_bc5` in `texture.rs` establish the current contract.
They verify dimensions plus compact oracle values without committing the oracle
images. Run them through the normal data-gated test path; they must skip when
the player's data, local key, or local mapping is unavailable and fail when
data is required by the full gates.

## Maintenance boundary

Do not generalize from these samples by accepting a new layout silently. Add a
separate source citation, structural validation, and data-gated regression test
for each new platform-data or pixel format. Upstream readers such as
[CUE4Parse](https://github.com/FabianFG/CUE4Parse) can be used as a comparison
oracle, but this project retains an independently implemented Rust reader and
does not vendor external reader code or game output.
