# Research evidence

This directory keeps **project-authored conclusions** needed to maintain the
readers and the offline simulation. It is not a copy of a player's game data
and is not an asset archive.

Read [../METHOD.md](../METHOD.md) before using or updating these notes. In
particular, `GAME_DATA` remains read-only, external runtime input; its assets,
keys, mappings, extracted packages, generated images, and conversion outputs
do not belong in this repository.

## What belongs here

Each note records only enough durable information to explain a project-owned
implementation:

- the supported wire-format or evidence boundary;
- the read-only `GAME_DATA` source used to establish a claim, expressed as a
  package path or a data-gated test rather than copied content;
- independently implemented validation invariants and known unsupported cases;
- the difference between an observation, a confirmed parser contract, and an
  unresolved hypothesis.

A path below is a citation relative to `GAME_DATA`, not a file included in the
repository. Do not add hashes of private inputs, raw bytes, decoded values,
keys, mappings, SDK dumps, screenshots, or extracted assets to make a note
more reproducible.

## Evidence status

Use these terms precisely:

| Term | Meaning |
| --- | --- |
| **Validated** | The project reader and a data-gated test establish the stated invariant on the cited input. |
| **Observed** | A read-only probe established the fact, but production code does not yet rely on it. |
| **Hypothesis** | A useful lead that must not determine gameplay or parsing without an independent check. |
| **Unsupported** | The current reader rejects the case rather than guessing. |

All findings are build-specific unless a note says otherwise. A new game build,
mapping, or container layout is new evidence, not a reason to silently loosen a
parser.

## Notes

| Note | Scope | Project implementation |
| --- | --- | --- |
| [TEX-FORMAT.md](TEX-FORMAT.md) | Virtual `UTexture2D` data and block-compressed mip-0 assembly | `crates/spdesc-data/src/texture.rs` |
| [MESH-FORMAT.md](MESH-FORMAT.md) | `USkeletalMesh` property boundary, render data, skeleton, and skin weights | `crates/spdesc-data/src/mesh.rs`, `skeletal.rs`, and `zen.rs` |
| [ANIM-FORMAT.md](ANIM-FORMAT.md) | ACL-compressed `UAnimSequence` clips and track-to-bone mapping | `crates/spdesc-data/src/anim.rs` and `crates/spdesc/src/content.rs` |
| [DATA-TABLE-SCHEMA.md](DATA-TABLE-SCHEMA.md) | Status and limits of data-table schema research | No production table reader yet |
| [MECHANICS-EVIDENCE.md](MECHANICS-EVIDENCE.md) | Client metadata evidence that can guide future local rules | `spdesc-game-core` remains the authority for game rules |

The historical probes, raw outputs, downloaded third-party material, and
extracted data remain local under the ignored `spdesc-config/` state root. They
are not a second documentation source to copy into Git.

## Updating a note

1. Establish a fact through a read-only `GAME_DATA` input or a bounded probe.
   Do not commit the input or probe output.
2. State the source, scope, validation, and limits. Separate what was observed
   from what was inferred.
3. Add or update a deterministic data-gated regression test when production
   code will rely on the fact. Tests must use the shared `GAME_DATA` resolver
   and report a skip when data is unavailable.
4. Record project-owned gameplay choices in the relevant rules or content
   schema, not as inferred facts about the original game.
5. Run the applicable repository checks before landing the change.

External formats and libraries may be cited for orientation, but their source
code and downloaded artifacts are not vendored here unless their licensing and
provenance are separately reviewed.
