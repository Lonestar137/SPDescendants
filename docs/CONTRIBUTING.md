# Contributing

Keep work small and verifiable: the `TFD_DATA` folder supplies the game's assets, formats, and
presentation and the optional `MW2_DATA` folder supplies the sound effects; both are read-only
runtime input, and simulation rules and content are project-owned. Do not add network/server
requirements or commit anything from a player's data folder.

## What gets in

- Behavior established by reading the file under the data folder that governs it, or by a probe
  that observes the produced behavior.
- Offline game simulation, persistence, and content imported through a documented,
  provenance-pinned conversion with deterministic tests.
- Fixes with a reproducible before/after behavior or a failing/passing test.
- Format support validated against real files without committing those files.

## What does not

- Runtime sockets, accounts, server processes, or external databases. Conversion tooling may
  read a documented source and produce a local content pack; that source is never a runtime
  dependency.
- Assets, extracted files, or code copied/derived from a data folder. Third-party *source* is a
  separate matter: it is carried with its licence and notice, recorded in `NOTICE.md`.
- A test that silently depends on a developer's data folder, a host clock, or nondeterministic
  RNG.
- Unplanned features outside the current slice.

## How a change is judged

1. The change is a coherent step, not a mixed rewrite, and stays within the current slice unless
   it is a necessary prerequisite.
2. Claims about data or presentation cite the file under the data folder that governs them or
   the probe that observed them. Project-owned rules state their design and provenance.
3. Tests cover the behavior deterministically. Runtime claims are backed by a bounded run or a
   numeric instrument, not only compilation or inspection.
4. Run `scripts/check.sh` for the round and `scripts/gates.sh` before the land.
5. Never write into a data folder; `scripts/data-guard.sh` fails the gates if anything under a
   resolved one is tracked.

## Setting up

- **First-time user setup.** Follow [`SETUP.md`](SETUP.md) for the data-folder path, local AES-key
  file, property mapping, first run, and troubleshooting. The project does not distribute those
  external local inputs.
- **Nix builds.** On a Nix system, always run builds, tests, clippy and repository check/gate
  scripts inside this repository's `shell.nix`, for example
  `nix-shell --run 'cargo test --workspace'`. The shell provides native dependencies such as
  ALSA and udev and exports `TFD_DATA` and `MW2_DATA`; do not fall back to a host `cargo`
  invocation.
- **Toolchain.** Use the stable Rust toolchain from `rust-toolchain.toml`, including rustfmt and
  clippy. `python3` is used by repository scripts.
- **Game data.** Provide your legally obtained install root through `TFD_DATA`, or symlink it as
  `TFD_DATA` beside the checkout (the dev link is gitignored and should point outside the repo).
  The root contains `M1/Content/Paks/`. `TFD_DATA=` explicitly means "no data" and is what the
  no-data tests use.
- **Sound data.** The sound effects come from an optional second install — Call of Duty: Modern
  Warfare 2 (2009) — through `MW2_DATA`, under the same rules (the dev link is gitignored).
  Without it the app falls back to its own synthesised cues and the sound tests skip.
- **Without data.** Unit tests and gates must remain useful. Data-dependent tests use
  `tfd_data_or_skip!()` or `mw2_data_or_skip!()` and report their skip; where a folder exists, that
  root's switch — `SPDESC_REQUIRE_DATA` or `SPDESC_REQUIRE_MW2`, set automatically by
  `scripts/gates.sh` — makes skips fail.
- **Local state.** Saves, settings, the AES key, and the property mapping resolve below
  `$SPDESC_HOME` (the checkout's ignored `spdesc-config/` under `nix-shell`); nothing else writes
  state. Never commit or share the key or mapping.
- **No server setup.** The game requires no account, server, realm, or network variables.
- **Development loop.** `cargo play` runs the app at release optimization with incremental
  compilation; `scripts/check.sh` gates a work round and `scripts/gates.sh` runs the full chain.

Working with an AI agent is expected. It reads `AGENTS.md`, and these rules bind it too.
