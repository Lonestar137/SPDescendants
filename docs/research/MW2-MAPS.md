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

## How a world's surfaces address the vertex pool

The world's surface array is 32-byte entries in the virtual block. Its relevant fields are:

- **`+4`, `firstVertex`** — the base in the shared world-vertex pool;
- **`+8`/`+10`, vertex/triangle counts** — the low/high halves respectively;
- **`+12`, `baseIndex`** — the first u16 index in the shared index buffer; and
- **`+16`, material** — a virtual-block pointer (present in 5,911 of 5,911 `mp_rust` surfaces).

The `baseIndex` runs tile the index buffer exactly, from zero to its end, and every run is a whole
number of triangles. More importantly, each u16 index in a run is **local**: its vertex is
`firstVertex + index`. In `mp_rust`, the first-vertex ranges partition the 235,386-record vertex pool
and every raw index fits its surface's vertex count. `vertexLayerData` at `+0` belongs to the separate
vertex-layer stream; it does not offset positions.

The independent check is the authored 24-byte bounds record beside each surface. It stores a midpoint
and half-size, so every resolved triangle vertex must lie inside it. The data-gated test in
`crates/spdesc-mw2/src/world.rs` verifies every index on `mp_rust`, `mp_subbase`, `mp_estate`, and
`mp_terminal` that the install ships (with 0.1 cm float tolerance). Direct indexing fails that check;
adding `firstVertex` passes it for every checked index.

## The drawn world

A vertex is 44 bytes and its position is the first three floats. `read_world` resolves every
surface-local index to the shared pool before returning the mesh, and the app now stands that drawn
world. The clipmap remains the source of the walkable-ground anchor, not the visual mesh.

The earlier collision-box test was the wrong invariant. A clipmap is the playable collision hull,
while the drawn world also contains non-collision geometry; their global extents therefore differ.
Per-surface authored bounds check the exact relationship the renderer needs without assuming that the
visual world and collision hull have the same outer box.

Measured: `mp_rust` is 5,911 surfaces over **76 materials**, and `mp_estate` 8,292 surfaces over
**222**; each run has 1 to 128 triangles. That is the grouping a renderer needs, and it shows that
the world's materials are named by its surfaces, not by the few map-level HUD material assets.

### Where a map stands

A map is stood in the world by its **walkable ground**, not by its lowest point. The collision flags
which triangles a player can stand on, and the spawn is the centre of the walkable triangle nearest
the middle of the walkable ground — a median, so a distant roof or catwalk cannot drag it away. The
map is then offset so that spot is at the origin, where the player already is.

Standing a map on its lowest point instead leaves any map with low ground at all — a valley, a
stairwell, a water channel — with its floors in the air above the player's feet, which is exactly
what it looked like in play before this.

## The unit: an inch, not a centimetre

A Modern Warfare 2 world unit is the engine's own — an **inch** — where The First Descendant works
in centimetres. The measure that settles it is the map's own doorways: histogramming near-vertical
collision edges (walls, door frames, ceilings) on `mp_terminal` puts a clear peak at **80–88
units**, and a Call of Duty doorway is 80 units. That is 2.0 m only as inches; as centimetres it is
80 cm, shorter than the player. Drawing a map at 0.01 m per unit therefore shrank the world to about
39% of its size, which is exactly why the player looked too big for it.

Every map position the app draws or places is converted with that unit
(`spdesc::content::mw2_to_bevy`), so the drawn world and the walkable-ground anchor agree with the
player's own centimetre frame.

## The collision, which reads whole

The clipmap is the map's collision hull: a vertex pool of 12-byte records that are three floats, and
triangle indices that are u16. Nothing has to be guessed, so it reads straight out. It supplies the
walkable-ground anchor for the drawn-world mesh.

| map | vertices | triangles | walkable | spans |
| --- | --- | --- | --- | --- |
| `mp_rust` | 43 489 | 68 408 | 17 584 | 1 519 × 2 344 × 90 m |
| `mp_estate` | 43 912 | 75 511 | 17 757 | 580 × 561 × 120 m |
| `mp_terminal` | 42 679 | 70 919 | 15 354 | 810 × 815 × 22 m |

The spans are the whole hull, which includes far-flung geometry; the ground the player actually
walks on is the map's playable shape, measured above in units rather than by its outer box. They are
a collision hull rather than the drawn world — no props or material detail — which is why they are
used for placement rather than displayed in place of the world surfaces.

## Standing and walking on a map

The player walks on the map, and its **floor and walls come from different places**.

The **floor** is the *drawn world's* surfaces. The clipmap's own triangle list is terrain, and on a
map built out of brushes it is not the map at all: Terminal carries **14 199 brushes** against
Rust's 2 668, and its triangle floors leave the terminal's interior bare — a walker fell through it
after ten metres. The drawn surfaces are continuous on every map and are the floor the player can
*see*, so that is the floor they stand on.

The **walls** are the clipmap's, because the drawn world's decoration — railings, pipes, cables —
would catch a player the game lets through. A live run on Rust with the drawn world's walls instead
had the walker pinned against a pipe for six seconds.

Both meshes are converted to the simulation's frame and units (the inch becomes centimetre), and
then two queries run every frame (`spdesc_game_core::collision`):

- **The floor** under the player is the highest floor-oriented triangle within a capsule's width of
  it and no more than a step (55 cm) above the feet, so a walker steps onto a stair or a kerb, climbs
  a slope, and falls off a ledge. A ceiling is not a floor.
- **The walls** push a vertical capsule out of anything beside it. A step short enough that the floor
  query would lift the walker onto it is *not* a wall, or stairs would be a wall — the first live run
  had the player pinned against a stair riser, moving two centimetres a second.

**Walking down uneven ground is walking, not falling.** Ground is not a plane, so a walker already on
it keeps it across a drop no deeper than a step (45 cm, `MovementParams::step_down`), and only a
short airtime (0.12 s) counts as a landing. Without both, every few centimetres of a slope left the
feet for a frame and played the landing — the character knelt its way across Rust.

A walker that leaves the map — off an edge with nothing below — falls past the hull's lowest point
and is put back on the spot the map stands around, so a map's void ends the fall.

**The map is stood on one of its own spawn points.** A map's entity text (`map_ents`) lists where the
game places a player — `mp_dm_spawn`, `mp_tdm_spawn`, `mp_dd_spawn_attacker` and the like, each with
an `origin` — so the anchor is a spawn, taken from the middle outwards, on the floor beneath it. That
is the map's own answer to "where does a player stand", which the clipmap's walkable flags do not
give: those name every floor, roof and stairwell alike.

What came before is worth recording because it looked reasonable and was worse. The anchor was first
chosen to *maximise floor coverage* — the level most grid spots stood on, then the busiest patch on
it — and it worked by its own measure and failed in play: it put the player on a large flat plane
*outside* the map, because the biggest, flattest floor is often not the playable one, and the
coverage figure was measured around the very spot it had chosen. Spawn points fixed the place; the
drawn world fixed the floor.

Measured afterwards — the share of a grid within thirty metres of the spawn with floor under it, from
the drawn world:

| map | floor coverage |
| --- | --- |
| `mp_rust` | 100% |
| `mp_terminal` | 100% |
| `mp_highrise` | 100% |
| `mp_favela` | 100% |
| `mp_subbase` | 82% |
| `mp_estate` | 53% |

Estate's figure is a manor of several levels: the grid is a flat slice, and a slice through a
staircase is half air. It is not the same as falling through.

### What the collision still is not

The clipmap's **brushes** — the convex solids most buildings' collision is made of — are not read.
Their planes are there (`brush_sides` names them) but turning a set of half-spaces into the solid a
capsule collides with is its own piece of work, and it is what the walls of a brush-heavy map want.
Until then a map's walls are its clipmap triangles, and where those run out a walker passes through.


## What is not read

- **Not material appearance.** The drawn-world mesh stands, grouped into one mesh per material, and
  each material is shown with a colour of its own so the surfaces can be told apart. The game's own
  images are not applied. The reason is a property of the fastfiles rather than a missing decoder:
  a map zone ships **no image assets at all** (`mp_rust` is 480 assets — a world, a clipmap, 245
  technique sets, 50 models, 143 sounds — and zero `image`), and its world's material bodies are not
  among the listed assets either (10 `material` assets, all HUD: the compass and the head icons).
  The world's 76 materials are addressed from another zone and resolved by the game's own linker, so
  reading them means loading the shared zones and emulating that resolution. The colour-per-material
  look is the interim that makes the world legible until then;
  [`MW2-TEXTURES.md`](MW2-TEXTURES.md) records that investigation — the images are in the `.iwd`
  archives and decode, and the one missing piece is which image each material names.
- **No localised titles.** `localized_*` zones carry no string table the zone walk captures, so
  `PATCH_VACANT` cannot yet be resolved to whatever the game's own text says. The key is kept on the
  entry (`title_key`) for the day it can.
- **No descriptions or overlays.** `description`, `mapoverlay` and the rest are read past.

## Instruments

`cargo run -p spdesc-mw2 --example sounds -- <install> <zone> --raw mp/basemaps.arena` prints a raw
file's text, and `--list <text>` prints the archive entry names matching it. Both were added while
establishing the above, and both are how a later map reader would check its own assumptions.
