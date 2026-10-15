# SPDescendants

An offline, single-player game in Rust and Bevy. The player's own game data supplies assets and
content; this project supplies the simulation and the code that presents it.

Read, in this order:

1. `docs/METHOD.md` — binding project rules and implementation method.
2. `docs/CONTRIBUTING.md` — scope, gates, and local setup.

The `TFD_DATA` folder — the game's own assets — and the optional `MW2_DATA` folder, which supplies
the sound effects, are runtime input only: both are read-only, and their files and anything derived
from them are never committed. The shipped game runs from the checkout and those folders — no
runtime sockets, accounts, server processes, or external databases.

@docs/METHOD.md
