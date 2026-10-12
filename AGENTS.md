# SPDescendants

An offline, single-player game in Rust and Bevy. The player's own game data supplies assets and
content; this project supplies the simulation and the code that presents it.

Read, in this order:

1. `docs/METHOD.md` — binding project rules and implementation method.
2. `docs/CONTRIBUTING.md` — scope, gates, and local setup.

The `GAME_DATA` folder is runtime input only: it is read-only, and its files and anything
derived from them are never committed. The shipped game runs from the checkout and that folder —
no runtime sockets, accounts, server processes, or external databases.

@docs/METHOD.md
