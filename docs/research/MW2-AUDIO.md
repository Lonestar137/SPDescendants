# MW2 audio: where the sound effects come from

Scope: the sound cues the app plays, and the reader behind them (`crates/spdesc-mw2`, with the
container vendored from IW4L — see `NOTICE.md` and `vendor/iw4/README.md`). Covers the PC build of
Call of Duty: Modern Warfare 2 (2009): IW4 fastfiles, zone version `0x114`, 64-bit wire layout.

This note is project-authored. It cites paths relative to `MW2_DATA` rather than including any.

## Why the effects are not loose files

`main/iw_*.iwd` are ZIP archives, and 18640 files under `Sound/` in them are plain `.wav`/`.mp3` —
but they are the wrong sounds. Measured over all of them:

| Directory | Files | Contents |
| --- | --- | --- |
| `Sound/battlechatter`, `Sound/voiceovers` | 17426 | dialogue |
| `Sound/music`, `Sound/ambient`, `Sound/amb_walla` | 292 | music and ambience |
| `Sound/amb_elements` | 442 | distant gunfire, not the player's weapon |
| `Sound/explosions`, `Sound/vehicles`, `Sound/level`, `Sound/doors`, `Sound/glass` | ~380 | scene and material effects |
| `Sound/weapons` | **1** | `grenade/weap_flashbang_tone.wav` |

No weapon fire, footstep, impact or interface cue is there. Those are *loaded* sounds: the audio
lives inside the fastfiles under `zone/english`.

## The container

A fastfile is a 21-byte file header over one zlib stream. Inflated, it is the zone image: an XFile
header (a size and eight block sizes) followed by every asset body, in the order the game loads
them. `crates/spdesc-mw2::open_zone` gets that far; `fastfile_iw4` reads the image.

The asset table at the start of the virtual block lists each asset as a `(type, pointer)` pair. Two
measured facts decide how the sounds have to be reached:

- **MW2 PC zones are the 64-bit wire layout.** The table validates against 8-byte pointers and not
  against 4, on all 96 zones under `zone/english`.
- **Every asset body is inline.** In `common.ff`, all 9288 entries — and all 2328 of the `Sound`
  type — carry a `Following` pointer, and the first `Sound` is entry 6707. A body is only at a known
  offset once everything before it has been loaded, so the sounds cannot be read out of the image
  directly.

So the reader walks the zone exactly as the game does: `load_asset_at_observed` per entry, in table
order, loading each body into the block its pointers name. It stops at the first asset type the
vendored reader has no layout for and reports where (`SoundSet::stopped_at`); everything before
that is captured. Both zones read here walked all the way through.

## The cues

Captured by walking `<MW2_DATA>/zone/english/common.ff` and `code_post_gfx.ff`:

| Zone | Assets walked | Aliases | Loaded payloads |
| --- | --- | --- | --- |
| `common.ff` | 9288 | 7570 | 959 |
| `code_post_gfx.ff` | 1972 | 18 | 10 |

The app reads both at startup — 7588 aliases, the startup log measuring ≈0.7 s on a warm page cache
— and takes each cue from the alias of the same name (`crates/spdesc/src/audio.rs`). Durations
below are the payload length divided by its own rate and channels, not the alias metadata:

| Cue | Alias | Payload |
| --- | --- | --- |
| shot | `weap_m4carbine_fire_plr` | stereo 44.1 kHz PCM, 1.344 s, one attack then a long tail |
| dry trigger | `weap_dryfire_rifle_plr` | mono 44.1 kHz, 0.279 s |
| hitmarker | `player_feedback_hit_alert` | mono 44.1 kHz, 0.034 s |
| target down | `bullet_impact_headshot` | mono 44.1 kHz, 0.813 s |
| reload begins | `weap_m4carbine_clipout_plr` | mono 44.1 kHz, 1.007 s, several mechanical transients |
| reload ends | `weap_m4carbine_clipin_plr` | mono 44.1 kHz, 0.653 s |
| footstep | `step_run_default` (walk/sprint differ only in name) | mono 44.1 kHz, 0.472 s |
| takeoff | `gear_rattle_plr_mantle` | mono 44.1 kHz, 0.753 s |
| touchdown | `Land_plr_default` | mono 44.1 kHz, 0.444 s |

The M4A1 is the weapon family used throughout (`weap_m4carbine_*`), because the app models an
assault rifle. A data-gated test in `crates/spdesc/src/audio.rs` walks the installed zone and fails
if any name above is missing or is not playable PCM, so the table cannot drift away from the data
silently.

## What the reader returns

`SoundSet` holds, for one zone:

- **`LoadedSound`** — an audio payload inside the zone: its name, the game's format code, rate,
  bits, channels and the encoded bytes. Format `1` is uncompressed 16-bit PCM, which
  `to_wav()` wraps in a RIFF header — the whole bridge to playback, since Bevy's `AudioSource`
  decodes WAV. `is_pcm()` and `duration_seconds()` are the same check.
- **`SoundAlias`** — one variant of an alias list: the name the game asks for, which payload or
  streamed file it names, and its authored mixing (looping, volume, pitch, distance, probability,
  sequence).

Only the payload's own name is used to match an alias to its audio; the alias's mixing values are
recorded but the app does not yet apply them.

## Limits

- **Only format 1 (16-bit PCM) plays.** Other format codes are captured but skipped by `to_wav`;
  959 of `common.ff`'s 7570 aliases carry a payload, and the test checks the named cues are among
  them rather than assuming any alias has audio.
- **Streamed aliases are recorded, not read.** 534 names in `common.ff` (mostly music and
  ambience) name a file in the `.iwd` archives instead of holding audio; `SoundAlias::streamed`
  keeps `(directory, file)` for a later reader.
- **The walk can stop early.** A zone holding an asset type the vendored reader has no layout for
  ends the walk there; `stopped_at` says which. Both zones used here completed.
- **MW2 has no jump cue.** No alias in either zone names one; takeoff uses the mantle foley, which
  is the closest authored motion sound.
- **No mixing.** Volumes are per-cue constants in `crates/spdesc/src/audio.rs`; the alias's own
  volume, pitch and distance values are not applied.
