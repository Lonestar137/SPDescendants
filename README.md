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

The game opens on its menu, and `Tab` opens and closes it after that. It has three pages, swapped
with `[` and `]`. **Map** chooses the ground to play on: the project's own arena, or a map read out
of the player's Modern Warfare 2 install, each drawn with its own preview art beside the list.
**Character** holds the four lists — character, body skin, head, face — and **Developer** the level
and prestige, the experience and prestige buttons, and the gun-fire and master volume sliders. Each
page's lists scroll, so nothing runs off the screen; the focused list is the one the arrows walk,
`PageUp`/`PageDown` jump a page and `Home`/`End` go to the ends. Every choice previews as it is
highlighted — a skin or a face on the character, a map's art beside its list — so nothing has to be
confirmed to be seen; `Enter`, `Escape`, `Tab` or the Close button all apply and close.

Outside the menu: `WASD`/arrows move, `Shift` sprints, `Space` jumps, the mouse orbits, right-click
aims, hold left-click to fire, and `R` reloads. The map being played is named along the top of the
screen.

**The practice targets stand on the project's own arena and nowhere else.** An imported map is
played without them, because a target is something this project put there rather than something a
map expects. Its world surfaces stand as a mesh, grouped by material and shown with a colour per
material, and the player is placed on one of the map's own spawn points and walks the drawn floor
with the clipmap's walls; the game's own textures, lightmaps, and material appearance remain to be
read. A Modern Warfare 2 world unit is an inch, so the map is drawn at its own scale rather than the
centimetre one the player works in. [`docs/research/MW2-MAPS.md`](docs/research/MW2-MAPS.md) records
what is and is not read.

Each target pays experience — a hit is worth 10 and a kill 100 — against Modern Warfare 2's own
rank table, so the bottom-left panel shows the level, the rank insignia, and how far the next rank
is. Reaching one rings the game's promotion fanfare; the ten prestige insignia are read from the
same install. **The level is kept between runs**, in `spdesc-config/career.json`, as are the sound
levels in `spdesc-config/settings.json`. `P` grants a chunk of experience and `O` takes the prestige
on offer, the same two things the menu's buttons do.

A model is built on a worker thread rather than in the frame: the menu asks for the one it is
showing and prefetches the ones a step away, so stepping through a list previews without waiting
where the neighbour was already built. The worker builds only the newest request in its queue, so a
fast run down a list collapses to wherever it stopped.

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
