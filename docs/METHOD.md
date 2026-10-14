# METHOD — how SPDescendants is built

SPDescendants is an offline, single-player game in Rust and Bevy. The player's own `GAME_DATA`
folder is the reference for assets, formats, and presentation. The project owns its simulation
rules and its content; there is no server and no network protocol on the game path.

## The rule for every change

Make one small, tested step toward the vertical slice. Use the player's data as evidence for
asset and presentation behavior. State project-owned gameplay decisions explicitly in the rules
or content schema. Keep the data folder read-only and external, keep the executable offline, and
run the appropriate gates.

Unless otherwise stated, consult the folder named by `GAME_DATA` as a source of truth, to make
sure a change matches the data it consumes.

## The loop

1. **Establish the fact.** For data, formats, or presentation, read the file under `GAME_DATA` or
   observe the behavior it produces. For game rules and content, record a project-owned decision
   and its provenance.
2. **Implement one slice.** Keep the game simulation (`spdesc-game-core`) independent of Bevy
   presentation. The app crate (`spdesc`) presents; it does not decide.
3. **Measure behavior.** Use deterministic tests, traces, and numeric probes. Visual captures are
   useful for appearance, not timing or ordering. Do not treat a successful build as evidence of
   runtime behavior.
4. **Verify the changed surface.** Run `scripts/check.sh` for a round and the full
   `scripts/gates.sh` before a land. A live run must prove the intended startup and gameplay path.
5. **Keep changes small.** Work on a branch, preserve unrelated worktree changes, and report the
   verification performed with each completed piece.

## Compile-efficient rounds

Treat a round as one cohesive, testable behavior slice, not one compile per edit. Before the
first build, trace the behavior through its relevant layers, decide which file under `GAME_DATA`
(or which probe) establishes the fact, identify the production and test files, and decide which
command will verify the whole slice. Then make the connected implementation and regression-test
edits before compiling. Do not bundle unrelated behavior just to amortize a build.

- On a Nix system, run every build, test, clippy, and check/gate command inside the repository's
  `nix-shell` so native dependencies and runtime libraries come from `shell.nix`.

- Prefer one test invocation naming all affected packages and the intended feature set. When
  packages share dependencies, this lets Cargo build their common graph once. Keep profiles and
  feature flags stable between runs so the artifacts are reusable.
- Avoid running `cargo check` and then the same tests solely as successive preflights; choose the
  check that answers the question for the round. A test build already type-checks its target.
- If a build or test fails, use its output to inspect and fix the whole failure set before the
  next build, unless a compile error or new evidence requires narrowing the implementation first.
- For repeated interactive presentation checks, prefer the workspace's incremental `play` profile
  (`cargo play`). Keep deterministic tests as the behavioral gate.
- Run `scripts/check.sh` once the cohesive round is ready, and the full land gates at the land
  step. Avoid cleaning build artifacts or changing profiles and feature combinations without a
  measured reason; either can turn a warm iteration into a cold build.

## Hard rules

- **Offline means no runtime transport or database.** The shipped executable must not open
  sockets, launch a server process, or require accounts, realms, or an external database.
- **The data folder is read-only runtime input.** Resolve it through `GAME_DATA` via
  `spdesc_game_core::game_data()` — the one resolver, never an ad-hoc path. Set and empty
  (`GAME_DATA=`) means "no data" on purpose; unset falls back to the `GAME_DATA` link beside the
  checkout in a dev build, and to `GAME_DATA`/`data` beside the binary in a shipped one. Never write into it.
- **Nothing from the data folder is committed.** No assets, no extracted files, no code copied or
  derived from its contents. It stays outside the repository by rule;
  `scripts/data-guard.sh` fails the gates when a resolved `GAME_DATA` sits inside the checkout
  and has tracked files, and `.gitignore` covers the default in-tree folder. Conversion tooling
  may read data and emit a project-owned content pack; record the source, revision, license, and
  conversion version with every imported set.
- **Local state has one root.** Saves and settings live under `spdesc-config/`. Per-character
  saves use stable generated IDs, not display names. Nothing else in the tree writes local state.
- **Simulation is deterministic.** `spdesc-game-core` owns game time and seeded randomness. Tests
  inject time and RNG state; gameplay does not depend on host wall-clock time.
- **Presentation follows Bevy.** `spdesc` presents what the simulation decides; it owns no game
  rules. Presentation-specific detail (layout, effects, camera) is its own concern.
- **Data-gated tests are honest.** A test that needs `GAME_DATA` uses `game_data_or_skip!()` and
  reports its skip. Where data resolves, `SPDESC_REQUIRE_DATA=1` (set by `scripts/gates.sh`)
  turns a skip into a failure. A test never silently depends on a developer's folder, a host
  clock, or nondeterministic RNG.
- **License reflects provenance.** Our code is MIT OR Apache-2.0; no assets or code from the
  player's data folder are bundled.

## Where things live

- `docs/METHOD.md`: binding engineering and provenance rules; never a work log.
- `docs/research/`: project-authored, provenance-safe evidence for accepted reader contracts;
  cite inputs and validation, but never include `GAME_DATA` artifacts or local configuration.
- `crates/`: Rust workspace. `spdesc-game-core` is the Bevy-free simulation (plus the data
  resolver); `spdesc` is the Bevy app that presents it.
- `scripts/`: gates and instruments (`check.sh` for a round, `gates.sh` at land).
- `.cargo/config.toml`: the `cargo play` alias; `Cargo.toml`: workspace dependencies and the
  `dev` / `ship` / `play` profiles.
- Git history records changes. Do not add progress logs or status narratives to the repository.

## Gates

The chain is: format, clippy with warnings denied, the data-guard, workspace tests with and
without data, and the no-data path. It runs green once per tree and stamps `target/.gates-green`,
so an unchanged tree re-greens without re-running. `GATES_FORCE=1` re-runs it.

Use `scripts/check.sh` for each implementation round; it scopes to the changed crates and their
dependents. Use `scripts/gates.sh` for the whole chain at land. A live run of the app is a
separate, named check — a green chain never claims one.
