# Reading Modern Warfare 2's map materials

A map's surfaces draw with **materials**, and a material's appearance is its images. This records
what was established about reaching them, what was tried, and where it stopped — so the next attempt
starts from the evidence rather than the beginning.

## What a map's surfaces name

The world's surface array is 32-byte entries in the virtual block (`docs/research/MW2-MAPS.md`). Each
surface's word at **`+16`** is the material it draws with; on `mp_rust` there are **76** distinct
values across 5 911 surfaces, and on `mp_terminal` **320** across 12 046. The value decodes as a
virtual-block offset (block 3), which is how the vendored reader follows it — `asset_ptr_at(Material,
…)` in `vendor/iw4/.../gfxworld.rs`.

## Where the images are

They are **not in the fastfiles**. `mp_rust`'s zone holds 480 assets — a world, a clipmap, 245
technique sets, 50 models, 143 sounds — and **zero `image` assets**; the 10 `material` assets it does
carry are the HUD (the compass, the head icons). Measured across the install: `common_mp` 0 images,
`localized_common_mp` 0, `estate` 0. Only `code_post_gfx_mp` carries any (7, the interface icons).

The map images are in the **`.iwd` archives** under `MW2_DATA/main/`, as `images/<name>.iwi` — the
same place the preview art and the rank icons come from. Listing them finds **12 074 image entries**,
of which **3 372** are `_col` (base colour). Spot-checked against names found inside a map zone:

```
images/rio_bg_vegitation01_col.iwi      (found in mp_rust's material region)
images/airport_compass_rose_col.iwi     (found in mp_terminal's material region)
```

So the images exist and are decodable (the `.iwi` reader already decodes DXT1/3/5 and the plain
formats). The missing piece is exactly one thing: **which image each surface's material names.**

## What is at the material offset

Reading the bytes a surface's `+16` points at does not give a `Material` as the vendored reader
expects one. On `mp_rust` the region is a long table of `{u64 0xFFFF…, u64 value}` entries, and it
sits beside a **name pool** of NUL-separated strings — `player_view_pitch_up`, `r_diffuseColorScale`,
`r_lightGridC…`, `ss_front_fx`, `tag_hood_fx`, `tag_glass_left_front` — which are dvar and model-tag
names, not material or image names.

The vendored `Material` layout *does* read the zone's own HUD materials correctly (name, texture
defs, technique set), so the layout is right for those. It simply is not what is at a *world*
surface's material offset.

## What was tried, and what it showed

- **Loading the material body at the offset.** `load_asset_at_observed` treats a `ZonePtr::Offset`
  as "already loaded" and returns without reading a body — so the world's materials are never read
  by the walk, and the offset points at whatever body the walk *did* allocate there.
- **Scanning from each material offset for image names.** Unbounded (4 KiB), 76/76 `mp_rust`
  materials "found" a `_col` name — but six consecutive materials all reported the *same* name
  (`rio_bg_vegitation01_col`), i.e. the scan was running into a shared pool. Bounding each scan by
  the *next* material's offset dropped it to **1/76**: the names are not inside a material's span.
- **Scanning the whole virtual block for `colorMap` definitions** (`41 10 ab a0 63 70` — the byte
  pattern the HUD materials' texture defs use — with an inline name): only **2–3** in the whole
  block, and they are not the map's materials.
- **Reading the offsets in `common_mp.ff`** (the shared multiplayer zone): floats and sound data, not
  materials.

## The likely shape of the answer

Modern Warfare 2 loads a map zone *together with* the shared zones, and a map's world materials are
resolved by the game's own linker at load — so on disk the surface's material word is a relocation
the reader cannot follow by itself. Either the materials' bodies live in another zone at a base this
reader does not compute, or the `Material` a world surface names has a layout this reader has not
seen (the HUD ones do not exercise it).

## The next steps, in order of promise

1. **Find how the material is meant to be resolved.** The vendored reader is derived from IW4L; if
   IW4L (or IW4x) loads MW2 multiplayer maps whole, its material handling is the missing rule —
   whether the materials are in a shared zone, or a `Material` field differs. This is the cheapest
   next step and the most likely to unblock it.
2. **Follow the `{0xFFFF…, value}` table properly.** If those entries *are* the world's material
   index — each a `{name pointer, name offset}` pair into the pool beside them — decoding the pair
   would give each material's *name*, and the name is what the `.iwd` archive indexes by.
3. **Only then** the reader: parse the material's texture defs for the `colorMap` argument, load
   `images/<name>.iwd`… `.iwi` from the archives (already possible), decode it, and bind one texture
   per surface run. The app's mesh is already grouped one child per material (`spdesc::map`), so
   binding is a small change once the name is known.

Until a name is known, the map keeps its current look: one colour per material, which at least tells
surfaces apart.
