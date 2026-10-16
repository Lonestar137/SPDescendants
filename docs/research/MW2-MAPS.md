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

## How a world's surfaces divide the index buffer

The world's surface array is 32-byte entries in the virtual block. Two of its words were established
from the data rather than from a remembered struct:

- **A material pointer at `+16`** — a virtual-block pointer in 5911 of 5911 surfaces of `mp_rust`.
- **A first index at `+12`** — the runs taken in that order tile the index buffer exactly, from 0 to
  the index count, and **every run is a whole number of triangles**. A partition derived from
  anything but the real first indices would not come out that way; the run *lengths* are therefore
  each surface's first index minus the next one's, and the last run ends at the index count.

Measured: `mp_rust` is 5 911 surfaces over **76 materials**, and `mp_estate` 8 292 surfaces over
**222**, each run 1 to 128 triangles. That is the grouping a render needs, and it also answers what
the earlier section left open: the world's materials are named by its surfaces, and there are
hundreds of them, not the nine material *assets* the map zone itself carries.

**Not established: how a run addresses its vertices.** A surface's `+4` word is too small to be the
run's first vertex in most surfaces, and using it as one produces a world extent of about a
kilometre, which is not a map. The vertex pool is also about four times the largest index in the
buffer, which fits the vertex-layer scheme the world's own `vertexLayerData` field implies. That,
and the position offset inside the 44-byte world vertex, is what a render needs next.

## The drawn world

A map's drawn geometry reads too. A surface states its **first index** at `+12` and its **triangle
count** as the high half of the word at `+8` — the low half is the world's own vertex count, which
every surface repeats. Measured against `mp_rust`: 5 911 surfaces, no two sharing a first index, no
run disagreeing with the count it states, and the counts covering the index buffer exactly
(579 864 of 579 864). The runs read earlier by taking each run's end from the next one's start were
right for the same reason — the counts confirm them.

A vertex is 44 bytes and its position is the first three floats. A triangle's indices are absolute
into the pool: a surface's `+4` word is zero for every surface seen, and the pool is addressed
directly. The pool is larger than the number of vertices addressed (235 386 against 58 496), which
is what suggested a per-surface base in the first place; it is not one.

Measured, the drawn world is bigger than the collision, as it should be: `mp_subbase` is 196 275
triangles over 308 × 311 × 86 m, `mp_estate` 219 239 over a manor and its grounds.

### Where a map stands

A map is stood in the world by its **walkable ground**, not by its lowest point. The collision flags
which triangles a player can stand on, and the spawn is the centre of the walkable triangle nearest
the middle of the walkable ground — a median, so a distant roof or catwalk cannot drag it away. The
map is then offset so that spot is at the origin, where the player already is.

Standing a map on its lowest point instead leaves any map with low ground at all — a valley, a
stairwell, a water channel — with its floors in the air above the player's feet, which is exactly
what it looked like in play before this.

## The collision, which reads whole

The clipmap is the one part of a map's *space* the vendored reader already captures completely: a
vertex pool of 12-byte records that are three floats, and triangle indices that are u16. Nothing has
to be guessed, so a map's collision reads straight out and stands in the world as one mesh.

| map | vertices | triangles | walkable | spans |
| --- | --- | --- | --- | --- |
| `mp_rust` | 43 489 | 68 408 | 17 584 | 598 × 923 × 35 m |
| `mp_estate` | 43 912 | 75 511 | 17 757 | 228 × 221 × 47 m |
| `mp_terminal` | 42 679 | 70 919 | 15 354 | 319 × 321 × 9 m |

Those are the map's own shapes: Terminal is a flat 9 metres tall, Estate a manor and its grounds.
It is the collision hull rather than the drawn world — no props, no detail, and the faces are wound
outward, so a camera inside reads the back of every wall — but it is the map, and it is what the
player will walk on.

## What is not read

- **Not the drawn world.** A map's *surfaces* — the geometry with the materials and lightmaps that
  make it look like a place — are read as runs (above) but not yet built as meshes. What stands in
  the world is the collision hull: the map's shape, untextured.
- **No localised titles.** `localized_*` zones carry no string table the zone walk captures, so
  `PATCH_VACANT` cannot yet be resolved to whatever the game's own text says. The key is kept on the
  entry (`title_key`) for the day it can.
- **No descriptions or overlays.** `description`, `mapoverlay` and the rest are read past.

## Instruments

`cargo run -p spdesc-mw2 --example sounds -- <install> <zone> --raw mp/basemaps.arena` prints a raw
file's text, and `--list <text>` prints the archive entry names matching it. Both were added while
establishing the above, and both are how a later map reader would check its own assumptions.
