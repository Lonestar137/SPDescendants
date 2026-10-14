# Skeletal-mesh format evidence

**Status:** validated for the character body and head inputs cited below. This
is the contract of the project's `USkeletalMesh` reader, not a claim that every
cooked Unreal mesh has this layout.

## Scope and provenance

The data readers in `crates/spdesc-data/src/mesh.rs`, `skeletal.rs`, and
`zen.rs` were established against these read-only `GAME_DATA` packages:

- `M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/001PC_004_U_CMN_BODY_001.uasset`
- `M1/Content/Characters/PC/MESH/004/U/Skel/SKIN/CMN/001PC_004_U_CMN_HEAD_001.uasset`

The packages, their inline bulk data, any exported glTF, and the local property
mapping remain outside Git. The cited package paths and the data-gated tests are
the evidence references; no mesh data is reproduced in this document.

## Export-range contract

Cooked export bytes normally begin at the package header boundary and follow
the export-map order. The reader validates every candidate range by parsing the
entire mesh export rather than trusting an offset alone.

One cited head package is a validated exception: its physical cooked exports
are ordered opposite to the relevant export-map entries. `ZenPackage` therefore
offers the ordinary and reverse-order ranges. The caller accepts a candidate
only when the mesh reader consumes it completely. This fallback is deliberately
narrow; it is not a general permission to reorder unknown exports.

## Property block and native tail

The `USkeletalMesh` property block is unversioned and needs a local schema to
establish its endpoint. The reader registers the native structures needed by
the tested chain, then starts the native tail exactly at that endpoint.

For the supported layout, the native tail contains, in order:

1. strip flags and imported bounds;
2. skeletal material slots;
3. the reference skeleton (bone names, parent indices, local bind transforms,
   and name-to-index data);
4. a cooked flag and LOD render data;
5. the small trailing LOD-count and object-array data.

The render data includes sections, index data, positions, tangents, UVs, skin
weights, and optional vertex colours. `FBoxSphereBounds` and `FTransform` use
the tested float representations in this layout. The reader treats every
array size and buffer boundary as untrusted input.

## Critical render-data invariants

These rules prevented convincing but incorrect geometry and are maintained by
the parser and its tests:

- A skin-weight record stores its bone indices before its weights; it is not an
  interleaved sequence.
- A weight index addresses the owning section's bone map, whose value is a
  reference-skeleton index. Section vertex ranges determine that ownership.
- Render indices in the tested LODs are global vertex indices. Do not add a
  section base vertex merely because that field exists.
- UV-buffer padding applies only to the non-tight element-count form.
- Lookup bulk metadata is read even when the reported lookup-vertex count is
  zero.
- Successful parsing requires all indices and joints to be in range, normalized
  skin weights, and full consumption of the selected export range.

The presentation layer converts the resulting project-owned `SkeletalMesh`
value to Bevy data; the parser itself does not decide rendering policy.

## Regression evidence

The data-gated tests in `mesh.rs` assert complete parsing of both cited meshes,
their expected high-level LOD/skeleton structure, in-range indices and joints,
normalized weights, and agreement between LOD bounds and imported bounds.
`skeletal.rs` separately proves that the property reader stops at the native
tail, while `zen.rs` proves that the package exposes the expected mesh export.

Those tests intentionally contain small oracle values rather than committing
packages or mesh exports. A changed build must either satisfy those structural
invariants or receive a separately documented, independently validated reader
path.

## Limits and update rules

This evidence does not validate cloth, morph targets, adjacency data, ray
tracing data, all skin-weight profiles, or arbitrary mesh packages. The head
range anomaly is a known special case, not an engine-wide fact.

When adding support for a new mesh form, cite its `GAME_DATA` package, retain
strict bounds and full-consumption checks, and add a data-gated test before
making it a fallback for existing inputs. Do not add the package, extracted
geometry, or an external tool's exported result to the repository.
