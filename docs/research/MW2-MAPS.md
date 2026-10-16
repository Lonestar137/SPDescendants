# Reading Modern Warfare 2's map list

The project's own map is the arena the practice targets stand in. Everything else it can offer comes
out of the player's `MW2_DATA`, and this records what was read, how, and what was left alone.

## The fact: `mp/basemaps.arena`

`code_post_gfx_mp.ff` — the same 188 KB multiplayer zone the rank tables come from — carries a raw
file `mp/basemaps.arena`, 968 bytes, zlib-compressed. Inflated, it is the game's base map rotation:

```
{
	map			"mp_invasion"
	longname	"MPUI_INVASION"
	gametype	"dm war sab sab2 dom sd sd2 hc thc ctf koth dd oneflag gtnw"
    description "MPUI_DESC_MAP_INVASION"
    mapimage    "preview_mp_invasion"
	mapoverlay  "compass_overlay_map_blank"
    allieschar  "us_army"
    axischar    "opforce_composite"
    environment	"urban"
}
```

Facts that shaped the reader:

- **Indentation is inconsistent.** Some lines are indented with tabs and others with spaces, so the
  file is a token walk, not a column parse.
- **Quoting is inconsistent.** Every value seen is quoted, but the format allows it not to be, so a
  bare word after a key is read as that key's value.
- **The map's name is its fastfile.** `zone/english/<map>.ff` is the map, which is why the app's
  check for "does this install have it" is `zone_path(install, map).is_file()`: the arena file lists
  the whole rotation, including maps this install does not ship.
- **The title is a key, not a name.** `longname` holds a localisation key. Two shapes appear in this
  one file — `MPUI_INVASION` and `PATCH_VACANT` — and they do not always resemble the map:
  `mp_nightshift` is keyed `MPUI_RAID`. So the app strips a known prefix and titles the rest, and
  falls back to the map's own name when the key names nothing.
- **Measured on this install:** 26 blocks, of which 16 ship a zone. The rest are downloadable maps
  whose zone is not present.

## The preview art

Each block names its own picture: `mapimage "preview_mp_invasion"`, which is
`images/preview_mp_invasion.iwi` in the `main/iw_*.iwd` archives. All 26 named previews are present
in the archives of this install, and all 16 shipped maps decode — verified by
`crates/spdesc/src/map.rs`'s preview test, which reads every one of them through the `.iwd` reader
and the `.iwi` decoder and checks the result is a picture rather than a flat colour.

They are read on their own thread, one walk of the archives for all of them: a few dozen images is
more than a frame should hold up, and reading each entry on its own would re-open an archive's
directory twenty-six times.

## What a map zone holds

A map is its own fastfile, and `cargo run -p spdesc-mw2 --example world` walks one and reports what
is in it. Measured on this install:

| map | assets | surfaces | vertices | indices | triangles | lightmap pages |
| --- | --- | --- | --- | --- | --- | --- |
| `mp_rust` | 480 | 5 911 | 235 386 | 579 864 | 193 288 | 3 |
| `mp_estate` | 797 | 8 292 | 284 403 | 657 717 | 219 239 | 2 |
| `mp_highrise` | 849 | 9 526 | 362 708 | 715 053 | 238 351 | 2 |
| `mp_terminal` | 817 | 12 046 | 408 705 | 959 670 | 319 890 | 1 |

Each zone walks in well under a second. The vertex and index pools are the world's own (every
surface draws out of them), so a map is 200k–320k triangles over 6k–12k surfaces, with one sky and
one to three lightmap pages — a few megabytes of vertex data, which is small for a GPU. The work in
rendering a map is therefore not volume: it is reading the surfaces, resolving what each one draws
with, and decoding the images.

The zones also hold the collision the player walks on (`clipmap`) and the map's own HUD art
(`material`: the compass, the objective icons). The materials the *world's surfaces* name are not
among them — resolving those is reading the surface array itself, which is the next step.

## What is not read

- **No map geometry is built.** Nothing here turns a map's world into meshes, so an imported map is
  *named* and *measured*, not built: the ground a game is played on is still the project's own
  arena, and the practice targets stand on it only when that arena is the map chosen. The counts
  above are what the vendored reader reports; they are not yet surfaces on screen.
- **No localised titles.** `localized_*` zones carry no string table the zone walk captures, so
  `PATCH_VACANT` cannot yet be resolved to whatever the game's own text says. The key is kept on the
  entry (`title_key`) for the day it can.
- **No descriptions or overlays.** `description`, `mapoverlay` and the rest are read past.

## Instruments

`cargo run -p spdesc-mw2 --example sounds -- <install> <zone> --raw mp/basemaps.arena` prints a raw
file's text, and `--list <text>` prints the archive entry names matching it. Both were added while
establishing the above, and both are how a later map reader would check its own assumptions.
