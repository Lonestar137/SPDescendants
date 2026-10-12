# Contributing

Keep work small and verifiable: the `GAME_DATA` folder is read-only runtime input for assets,
formats, and presentation; simulation rules and content are project-owned. Do not add
network/server requirements or commit anything from a player's data folder.

## What gets in

- Behavior established by reading the file under `GAME_DATA` that governs it, or by a probe that
  observes the produced behavior.
- Offline game simulation, persistence, and content imported through a documented,
  provenance-pinned conversion with deterministic tests.
- Fixes with a reproducible before/after behavior or a failing/passing test.
- Format support validated against real files without committing those files.

## What does not

- Runtime sockets, accounts, server processes, or external databases. Conversion tooling may
  read a documented source and produce a local content pack; that source is never a runtime
  dependency.
- Assets, extracted files, or code copied/derived from `GAME_DATA`.
- A test that silently depends on a developer's data folder, a host clock, or nondeterministic
  RNG.
- Unplanned features outside the current slice.

## How a change is judged

1. The change is a coherent step, not a mixed rewrite, and stays within the current slice unless
   it is a necessary prerequisite.
2. Claims about data or presentation cite the file under `GAME_DATA` or the probe that observed
   it. Project-owned rules state their design and provenance.
3. Tests cover the behavior deterministically. Runtime claims are backed by a bounded run or a
   numeric instrument, not only compilation or inspection.
4. Run `scripts/check.sh` for the round and `scripts/gates.sh` before the land.
5. Never write into the data folder; `scripts/data-guard.sh` fails the gates if anything under a
   resolved `GAME_DATA` is tracked.

## Setting up

- **Nix builds.** On a Nix system, always run builds, tests, clippy and repository check/gate
  scripts inside this repository's `shell.nix`, for example
  `nix-shell --run 'cargo test --workspace'`. The shell provides native dependencies such as
  ALSA and udev and exports `GAME_DATA`; do not fall back to a host `cargo` invocation.
- **Toolchain.** Use the stable Rust toolchain from `rust-toolchain.toml`, including rustfmt and
  clippy. `python3` is used by repository scripts.
- **Game data.** Provide your legally obtained data folder through `GAME_DATA`, or symlink it
  as `GAME_DATA` beside the checkout (the dev link, gitignored — it points outside the repo).
  `GAME_DATA=` explicitly means "no data" and is what the no-data tests use.
- **Without data.** Unit tests and gates must remain useful. Data-dependent tests use
  `game_data_or_skip!()` and report their skip; where data exists, `SPDESC_REQUIRE_DATA=1`
  (set automatically by `scripts/gates.sh`) makes skips fail.
- **Local state.** Saves and settings resolve to `spdesc-config/` (`$SPDESC_HOME` under
  `nix-shell`); nothing else writes state.
- **No server setup.** The game requires no account, server, realm, or network variables.
- **Development loop.** `cargo play` runs the app at release optimization with incremental
  compilation; `scripts/check.sh` gates a work round and `scripts/gates.sh` runs the full chain.

Working with an AI agent is expected. It reads `AGENTS.md`, and these rules bind it too.
