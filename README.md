# SPDescendants

An offline, single-player game in Rust and [Bevy](https://bevy.org). The player's own game data
supplies assets and content — The First Descendant's install for the game itself, and optionally
Call of Duty: Modern Warfare 2 for the sound effects; this project supplies the simulation and the
code that presents it.

- `docs/METHOD.md` — binding rules (read the data folder read-only, keep the executable
  offline, keep the simulation deterministic).
- `docs/SETUP.md` — first-run setup for game data, the local AES key, and the property mapping.
- `docs/CONTRIBUTING.md` — scope, gates, and development setup.

## Layout

| Path | What it is |
| --- | --- |
| `crates/spdesc-game-core` | Bevy-free simulation: rules, content, persistence, and the data-folder resolvers |
| `crates/spdesc-data` | Reading `TFD_DATA`: the IoStore container and the packages inside it |
| `crates/spdesc-mw2` | Reading `MW2_DATA`: Modern Warfare 2 fastfiles, for the sound effects |
| `crates/spdesc` | The Bevy app that presents the simulation |
| `scripts/check.sh` | In-loop verify: fmt + clippy + tests scoped to the changed crates |
| `scripts/gates.sh` | The land chain: fmt, clippy, data-guard, tests with and without data |
| `Cargo.toml` | Workspace deps and the `dev` / `ship` / `play` profiles |

## Quick start

For a full character model, first follow [First-run setup](docs/SETUP.md): provide your own
legally obtained game install, its AES-256 key, and a compatible property mapping. None of those
files are distributed with this repository. The app can still start without them, using a capsule
instead of the game character.

```sh
TFD_DATA="/path/to/The First Descendant" nix-shell # enter dev shell with your install selected
cargo play                                         # from inside nix-shell
scripts/check.sh                   # gate the round
scripts/gates.sh                   # full chain before a land
```

`TFD_DATA` must be the install root containing `M1/Content/Paks/`, not the `Paks` directory
itself. A gitignored `TFD_DATA` symlink beside the checkout also works. To run without game data,
set `TFD_DATA=` explicitly for a command; an empty value means “no data,” not “use the default.”

The sound effects are read from **Modern Warfare 2** instead: point `MW2_DATA` at that install (the
folder holding `main/` and `zone/`), or symlink it beside the checkout the same way. It is
optional — without it, the app falls back to its own synthesised cues and the sound tests skip.

## Playing

`Tab` opens/closes the selection menu. Use left/right to choose the character, body-skin, head, or
face column; use up/down to change its selection; press `Enter` to apply, or `Escape` to close.
Otherwise: `WASD`/arrows move, `Shift` sprints, `Space` jumps, the mouse orbits, right-click aims,
hold left-click to fire, and `R` reloads.

Each target pays experience — a hit is worth 10 and a kill 100 — against Modern Warfare 2's own
rank table, so the bottom-left panel shows the level, the rank insignia, and how far the next rank
is. Reaching one rings the game's promotion fanfare; the ten prestige insignia are read from the
same install. `P` grants a chunk of experience and `O` takes the prestige on offer once the last
rank is held, which is the quick way to walk the whole system.

Character names come from `spdesc-config/roster.json` (a small id → name index, like the key and
property mapping beside it); without it characters are labelled by their id.

The sound effects — the shot, the hitmarker, footsteps, jumping and landing, the reload — are
Modern Warfare 2's own, read from `MW2_DATA` when it is there and synthesised otherwise. To see
what an install offers, or to write the audio out and listen to it:

```sh
cargo run -p spdesc-mw2 --example sounds -- --match gunfire
cargo run -p spdesc-mw2 --example sounds -- /path/to/mw2 common --dump /tmp/mw2-cues
```

## Rules that matter most

- **A data folder is read-only runtime input.** Resolve `TFD_DATA` through
  `spdesc_game_core::tfd_data()` and `MW2_DATA` through `spdesc_game_core::mw2_data()`; never write
  into either, never commit anything from either (`scripts/data-guard.sh` enforces the last part).
- **Nothing from a data folder is committed** — no assets, no extracted files, no code derived
  from them. Third-party source is carried with its licence and notice (`NOTICE.md`).
- **Local state has one root:** `spdesc-config/`.
- **Offline:** no runtime sockets, accounts, servers, or external databases.
- **Deterministic simulation:** `spdesc-game-core` owns game time and seeded randomness.
