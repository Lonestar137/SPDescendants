# NOTICE

SPDescendants is licensed under MIT OR Apache-2.0 (`Cargo.toml`, `[workspace.package]`). It
bundles no game assets and no code derived from a player's data folder.

## Vendored third-party source

`vendor/iw4/` carries four crates copied from **IW4L**, an open-source runtime for Call of Duty:
Modern Warfare 2 (2009):

> IW4L — Copyright 2026 vladtrc — <https://github.com/vladtrc/iw4L> — Apache License 2.0.

| Vendored crate | Upstream crate | What it does |
| --- | --- | --- |
| `vendor/iw4/fastfile_iw4` | `fastfile_iw4` | The IW4 zone container: file header, stream, pointer fixup, asset table, asset bodies. |
| `vendor/iw4/asset_iw4` | `asset_iw4` | Retail IW4 asset layouts: struct sizes, sound-alias fields, material and vertex-declaration forms. |
| `vendor/iw4/d3d9_decl` | `d3d9_decl` | D3D9 vertex-declaration types. |
| `vendor/iw4/d3d9_state` | `d3d9_state` | D3D9 render-state and sampler value semantics. |

Taken from upstream commit `f026f5f` (2026-10-05). The source is unmodified apart from a provenance
comment at the top of each crate's `lib.rs` and the crate manifests, which spell out what they used
to inherit and make each crate its own workspace root so this repository's `cargo fmt` and
`clippy` do not restyle or relint third-party source. `vendor/iw4/README.md` records the
provenance; `docs/research/MW2-AUDIO.md` records what the project uses them for.

The vendored crates live outside the workspace `members` list on purpose: they are third-party
source, so this repository's `cargo fmt` and `clippy -D warnings` do not restyle or relint them.

No other third-party source is vendored. Every dependency compiled from crates.io is listed in
`Cargo.lock` under its own licence.
