# SPDescendants

An offline, single-player game in Rust and [Bevy](https://bevy.org). The player's own game data
supplies assets and content; this project supplies the simulation and the code that presents it.

- `docs/METHOD.md` — binding rules (read the data folder read-only, keep the executable
  offline, keep the simulation deterministic).
- `docs/CONTRIBUTING.md` — scope, gates, and local setup.

## Layout

| Path | What it is |
| --- | --- |
| `crates/spdesc-game-core` | Bevy-free simulation: rules, content, persistence, and the `GAME_DATA` resolver |
| `crates/spdesc` | The Bevy app that presents the simulation |
| `scripts/check.sh` | In-loop verify: fmt + clippy + tests scoped to the changed crates |
| `scripts/gates.sh` | The land chain: fmt, clippy, data-guard, tests with and without data |
| `Cargo.toml` | Workspace deps and the `dev` / `ship` / `play` profiles |

## Quick start

```sh
nix-shell                          # toolchain + native libs, exports GAME_DATA
cargo play                         # release optimization + incremental (cargo alias)
GAME_DATA=/path/to/data cargo play # an explicit folder; `GAME_DATA=` means "no data"
scripts/check.sh                   # gate the round
scripts/gates.sh                   # full chain before a land
```

## Playing

`Tab` opens the character select screen: pick a character on the left, one of its skins on the
right, and `Enter` to rebuild the model in place (the cursor is released while the menu is open).
Otherwise: `WASD`/arrows move, `Shift` sprints, `Space` jumps, the mouse orbits, left-click fires,
and `R` reloads.

Character names come from `spdesc-config/roster.json` (a small id → name index, like the key and
property mapping beside it); without it characters are labelled by their id.

## Rules that matter most

- **`GAME_DATA` is read-only runtime input.** Resolve through
  `spdesc_game_core::game_data()`; never write into it, never commit anything from it
  (`scripts/data-guard.sh` enforces the last part).
- **Nothing from the data folder is committed** — no assets, no extracted files, no code derived
  from them.
- **Local state has one root:** `spdesc-config/`.
- **Offline:** no runtime sockets, accounts, servers, or external databases.
- **Deterministic simulation:** `spdesc-game-core` owns game time and seeded randomness.
