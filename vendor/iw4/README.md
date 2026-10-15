# Vendored IW4L crates

Four crates copied verbatim from [IW4L](https://github.com/vladtrc/iw4L), an open-source runtime
for Call of Duty: Modern Warfare 2 (2009), Apache License 2.0.

They are the container half of reading a Modern Warfare 2 fastfile: without them, the sound
effects in `MW2_DATA` cannot be reached (see `docs/research/MW2-AUDIO.md` for why). `crates/spdesc-mw2`
is the project's own code on top of them; nothing else in this workspace depends on them.

## Provenance

| | |
| --- | --- |
| Upstream | <https://github.com/vladtrc/iw4L> |
| Commit | `f026f5fca0b9e3550f6c053a577caff67b3d3dc9` (2026-10-05) |
| Licence | Apache-2.0 (`LICENSE`, `NOTICE.md`) |
| Copied | `crates/fastfile_iw4`, `crates/asset_iw4`, `crates/d3d9_decl`, `crates/d3d9_state` |

## What changed on the way in

Only what a crate outside its own workspace needs:

- each `Cargo.toml` spells out `edition` and `license` rather than inheriting them, drops the
  workspace `[lints]` table, takes its siblings by relative `path`, and declares an empty
  `[workspace]` so each crate is its own workspace root — with the root manifest's `exclude` list,
  that keeps this repository's `cargo fmt` and `clippy` off third-party source;
- each `lib.rs` opens with a provenance comment.

Everything under `src/` is otherwise upstream source, unmodified.

## Re-syncing

Replace a crate directory from a newer upstream checkout, re-apply those two edits, and update the
commit above. The vendored crates are deliberately absent from the workspace `members` list, so
`cargo fmt --all` and `cargo clippy --workspace` leave them alone; keep it that way and do not add
project-specific code here.
