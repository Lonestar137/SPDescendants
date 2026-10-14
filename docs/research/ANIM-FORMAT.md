# Animation format evidence

**Status:** validated for the ACL path used by the cited character clips. This
note records the project's decoder contract and animation-to-skeleton mapping;
it is not a generic `UAnimSequence` implementation.

## Scope and provenance

`crates/spdesc-data/src/anim.rs` reads `compressed_tracks` data from cooked
animation exports. The principal data-gated regression input is:

- `M1/Content/Characters/PC/Anim/F/CMN/Sequence/IdleF_0LT_Idle.uasset`

Track-to-bone mapping also reads the character skeleton at:

- `M1/Content/Characters/PCPC_Skeleton.uasset`

The packages, clip payloads, decoded transforms, local key, and mapping files
remain outside Git. The tests cite the inputs through the normal `GAME_DATA`
resolver and contain only compact oracle values needed to catch reader drift.

## Codec identification and blob location

The tested clips use the Unreal ACL plugin's
`AnimBoneCompressionCodec_ACL` codec. Their payload is an
[Animation Compression Library](https://github.com/nfrechette/acl)
`compressed_tracks` blob at ACL format version 7 (ACL 2.0.0).

The project does not assume a fixed offset for that blob. It scans an animation
export for the ACL tag, then accepts a candidate only when its raw-buffer header
is self-consistent and its version is supported. This avoids treating unrelated
export bytes as animation data.

## Supported ACL contract

The decoder supports transform tracks with uniform sampling, including one or
more segments, default/constant/animated rotation, translation, and scale
sub-tracks. It reads:

- the tracks header and transform-track metadata;
- segment headers and segment start indices;
- packed two-bit sub-track kinds;
- constant track data and clip/segment ranges; and
- variable-width animated samples.

Variable samples are bit-packed most-significant-bit first. Quantized triples
are normalized and remapped through the recorded ranges. Compressed rotations
that omit W reconstruct its positive root. The implementation is scalar and
from scratch; ACL is an offline comparison oracle, not a runtime dependency.

The reader rejects instead of guessing for unsupported ACL features, including
database streaming, metadata, stripped keyframes, non-transform tracks, and
unknown versions or formats.

## Mapping a clip to a mesh

A clip's track order is not the mesh bone order. The mapping is:

```text
clip track index
    -> CompressedTrackToSkeletonMapTable entry
    -> ordered reference-skeleton bone
    -> mesh bone with the same name
```

The ordered skeleton is structurally recovered from the cited `USkeleton`
export; each clip's map is independently recovered from its own export. A clip
can animate only a subset of the skeleton, and different clips can select
different subsets, so callers must keep one mapping per clip. Unmapped mesh
bones retain their bind pose.

`crates/spdesc/src/content.rs` owns the presentation-side conversion of these
transforms and `crates/spdesc/src/player.rs` applies the model-space yaw offset.
The model's authored forward axis is `+Y`, while the project's simulation
forward axis is `+X`; that quarter-turn is a presentation convention, not a
simulation rule.

## Regression evidence

`idle_clip_matches_the_acl_oracle` validates the principal clip's dimensions,
sample rate, and transforms sampled across the clip against values produced by
ACL 2.0.0. A prior local read-only comparison also observed agreement across
idle, locomotion, jump, turn, and weapon clips; that broader result is useful
context, not a substitute for repository regression coverage. Future decoder
changes must retain a representative data-gated oracle test and should add a
clip when they support a new encoding case.

The test is intentionally skipped when player-owned data or required local
configuration is absent. With `SPDESC_REQUIRE_DATA=1`, the repository gates
turn such a skip into a failure, preventing a claimed data-path validation from
quietly disappearing.

## Limits and update rules

Do not infer a codec from an animation's class name alone, and do not use an
asset-specific byte offset as a parser rule. A new codec or ACL feature needs a
separate source citation, structural validation, and a regression oracle before
the runtime accepts it. Do not commit a clip, an extracted skeleton, or ACL
source code as part of that work.
