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

## What is not read

- **No map geometry.** Nothing here parses a `.ff` map's BSP, its worldspawn, its lightmaps or its
  materials, so an imported map is *named*, not built: the ground a game is played on is still the
  project's own arena, and the practice targets stand on it only when that arena is the map chosen.
- **No localised titles.** `localized_*` zones carry no string table the zone walk captures, so
  `PATCH_VACANT` cannot yet be resolved to whatever the game's own text says. The key is kept on the
  entry (`title_key`) for the day it can.
- **No descriptions or overlays.** `description`, `mapoverlay` and the rest are read past.

## Instruments

`cargo run -p spdesc-mw2 --example sounds -- <install> <zone> --raw mp/basemaps.arena` prints a raw
file's text, and `--list <text>` prints the archive entry names matching it. Both were added while
establishing the above, and both are how a later map reader would check its own assumptions.
