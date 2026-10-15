# First-run setup

SPDescendants is an offline game, but its character assets come from **your own legally obtained
game install**. The checkout does not include the game, an AES key, or a property mapping. Those
inputs stay on your machine under the ignored `spdesc-config/` folder; do not add them to Git or
send them in an issue, chat, or screenshot.

## 1. Enter the development environment

From the repository root, on a Nix system:

```sh
nix-shell
```

The shell provides the native libraries and stable Rust environment used by the project. It also
sets `SPDESC_HOME` to `<checkout>/spdesc-config` and, for each data folder that was not already
set, defaults `TFD_DATA` to `<checkout>/TFD_DATA` and `MW2_DATA` to `<checkout>/MW2_DATA`.

## 2. Point `TFD_DATA` at the install

Set `TFD_DATA` to the **install root** whose contents include
`M1/Content/Paks/M1-Windows.utoc` and `M1-Windows.ucas`—not to the `Paks` directory itself. You
can pass the path before entering Nix:

```sh
TFD_DATA="/absolute/path/to/The First Descendant" nix-shell
```

Or, from inside the shell, set it before starting the app or tests:

```sh
export TFD_DATA="/absolute/path/to/The First Descendant"
```

As an alternative, create a gitignored symlink from the repository root to the external install:

```sh
ln -s "/absolute/path/to/The First Descendant" TFD_DATA
nix-shell
```

Check that the path is the correct root:

```sh
test -f "$TFD_DATA/M1/Content/Paks/M1-Windows.utoc" \
  && test -f "$TFD_DATA/M1/Content/Paks/M1-Windows.ucas"
```

The game-data directory is read-only runtime input. Do not copy the install into the checkout or
write extracted/conversion output into it. To deliberately run without data, set `TFD_DATA=` for
the command; an explicitly empty variable disables the fallback paths.

## 3. Optional: point `MW2_DATA` at a Modern Warfare 2 install

The sound effects — the shot, the hitmarker, footsteps, jumping and landing, and the reload — come
from **Call of Duty: Modern Warfare 2 (2009)** rather than from The First Descendant. This step is
optional: without it the app synthesises its own cues instead, and tests that need the sounds
report a skip.

Point `MW2_DATA` at the install root — the folder whose contents include `main/` and `zone/` — with
the same three options as above:

```sh
MW2_DATA="/absolute/path/to/Call of Duty Modern Warfare 2" nix-shell
```

```sh
export MW2_DATA="/absolute/path/to/Call of Duty Modern Warfare 2"
```

```sh
ln -s "/absolute/path/to/Call of Duty Modern Warfare 2" MW2_DATA
nix-shell
```

Check that the path is the correct root:

```sh
test -d "$MW2_DATA/main" && test -d "$MW2_DATA/zone/english"
```

The folder is read-only runtime input, exactly like `TFD_DATA`: the app reads the fastfiles in it
and never writes into it. No key or mapping is needed for it. Besides the sound effects it supplies
the rank table and the rank and prestige insignia, so the level and prestige system needs this
folder too — without it the career and its HUD stay hidden.

## 4. Add your AES key to local state

The reader expects a 256-bit AES key as **64 hexadecimal characters** in
`$SPDESC_HOME/keys.json`, under the `aes256_hex` field. The field may have a `0x` prefix, but plain
hex is simplest. Use a key you are legally entitled to use with your own install; this repository
does not provide, recover, or derive one.

While in the development shell, create the file with a text editor:

```sh
mkdir -p "$SPDESC_HOME/mappings"
${EDITOR:-nano} "$SPDESC_HOME/keys.json"
```

Use this shape, replacing the placeholder with your own 64 hexadecimal characters:

```json
{
  "aes256_hex": "REPLACE_WITH_YOUR_64_HEX_DIGIT_KEY"
}
```

The placeholder is not a working key. On Unix-like systems, restrict access to the file:

```sh
chmod 600 "$SPDESC_HOME/keys.json"
```

Do not put the key in a command-line argument, source file, test, issue, or commit. The project
ignores `spdesc-config/`, but still check `git status` before committing local changes.

## 5. Install a compatible property mapping

The cooked packages use unversioned properties, so the asset reader also needs a mapping that
matches the game build. Put it at:

```text
$SPDESC_HOME/mappings/usmap-fmodel-structs.json
```

The reader expects FModel/UAssetAPI-style **JSON**: an object of struct definitions with ordered
`props` and optional `super` fields. It does not read a raw binary `.usmap` file at this path. The
mapping is player-supplied and build-specific; this repository does not include one. Use a mapping
you are entitled to use, exported/converted to the expected JSON format, and keep it local. An
incompatible mapping can fail parsing even if its JSON syntax is valid.

The local configuration should then look like:

```text
spdesc-config/
├── keys.json
└── mappings/
    └── usmap-fmodel-structs.json
```

Under `nix-shell`, `$SPDESC_HOME` points to that checkout-local folder. When running the app
outside this shell, set `SPDESC_HOME` explicitly (for example,
`SPDESC_HOME="$PWD/spdesc-config"`). The app disables the source-tree fallback so a shipped binary
cannot depend on a developer's checkout path.

## 6. Run the game and checks

With `TFD_DATA`, key, and mapping configured (and `MW2_DATA` if you want the gunshot and foley
sounds), run inside `nix-shell`:

```sh
cargo play
```

`Tab` opens/closes the selection menu. Use left/right to choose the character, body-skin, head, or
face column; use up/down to change the selection; press `Enter` to apply or `Escape` to close.
Outside the menu, `WASD` or the arrow keys move, `Shift` sprints, `Space` jumps, the mouse orbits,
right-click aims, hold left-click to fire, and `R` reloads. `F1` opens the developer menu: the
level and prestige, a button to reset the prestige, and sliders for the gun-fire and master
volumes.

Run the in-loop checks for a change, and the full gate chain before landing it:

```sh
scripts/check.sh
scripts/gates.sh
```

To run tests intentionally without any data folder, empty both variables explicitly:

```sh
TFD_DATA= MW2_DATA= cargo test --workspace
```

Tests that need data files report a skip in this mode. With a folder resolved, `scripts/gates.sh`
sets that root's switch — `SPDESC_REQUIRE_DATA` or `SPDESC_REQUIRE_MW2` — so a data test cannot
silently skip the data that is actually installed.

## What the game keeps

Everything the game keeps between runs is a small JSON document under `$SPDESC_HOME`, written when
it changes and not otherwise:

| File | Holds |
| --- | --- |
| `selection.json` | the character, body skin, head and face last applied |
| `settings.json` | the gun-fire and master volumes |
| `career.json` | the level, the prestige, and the experience inside the rank |

Each stands alone: a missing or malformed file means the default, never a half-read state, so
deleting one is a safe way to start that part over. The career is the player's, not a character's,
which is how the game's own progression works.

## Optional local character names

`$SPDESC_HOME/roster.json` maps character IDs to display names. It is optional; without it, the
selection screen displays IDs. For example:

```json
{
  "004": "Bunny"
}
```

## Troubleshooting

- **No character model / capsule fallback:** check the app log, `TFD_DATA`, `keys.json`, and the
  mapping path and format. The app can start without these inputs, but cannot load game assets.
- **“No TFD_DATA folder found”:** point `TFD_DATA` at the install root and confirm the two
  `M1/Content/Paks/` files above exist there.
- **“No container key”:** confirm `$SPDESC_HOME/keys.json` is valid JSON and its `aes256_hex` value
  is exactly 64 hexadecimal characters (optionally preceded by `0x`). A syntactically valid but
  wrong key will still fail to decrypt the container.
- **“No property mapping” / parse errors:** confirm the exact mapping filename, JSON export format,
  and compatibility with the installed game build.
- **No MW2 sounds / synthesised cues:** check that `MW2_DATA` resolves to the Modern Warfare 2
  install root and that `zone/english/` holds `common.ff`. The app works without it, so a log line
  saying the sounds were not found is not an error. The startup line names what was read
  (`sounds: 7588 aliases from 2 zone(s) of … in 0.7 s`), and the reader can be exercised on its own:
  `cargo run -p spdesc-mw2 --example sounds -- --match footstep`.
- **Data-gated tests skip:** check that the data folder the test names resolves to its install root
  and that the local key and mapping are available. On Nix systems, rerun the command inside this
  repository's `nix-shell`.

No server, account, or network service is part of the game path. See
[`CONTRIBUTING.md`](CONTRIBUTING.md) for project rules and
[`research/README.md`](research/README.md) for the evidence behind supported data readers.
