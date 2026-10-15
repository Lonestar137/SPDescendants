# MW2 progression: ranks, experience and prestige

Scope: the level and prestige system the app presents — the rank table, the insignia art, and the
cues that announce them — and where each comes from. Covers the PC build of Call of Duty: Modern
Warfare 2 (2009). The rules live in `spdesc-game-core::progression`; the data is read by
`crates/spdesc-mw2`; `crates/spdesc/src/progress.rs` joins them.

This note is project-authored. It cites paths relative to `MW2_DATA` rather than including any.

## The tables are the game's own

`zone/english/code_post_gfx_mp.ff` is 188 KB and holds the multiplayer tables. Read by walking it:
292 assets, 77 string tables. Two of them are the progression system:

| Table | Shape | Holds |
| --- | --- | --- |
| `mp/ranktable.csv` | 18 columns × 72 rows | one row per rank, plus a header and a `maxrank` row |
| `mp/rankicontable.csv` | 12 columns × 72 rows | one row per rank, naming its insignia and the ten prestige insignia |

The columns of `mp/ranktable.csv`, measured from its rows:

| Column | Meaning | Rank 1 (`pvt1`) | Rank 70 (`comm1`) |
| --- | --- | --- | --- |
| 0 | rank index, from zero | 0 | 69 |
| 1 | the rank's name | `pvt1` | `comm1` |
| 2 | experience needed to reach the rank | 0 | 2 434 700 |
| 3 | experience needed inside it to leave | 500 | 81 300 |
| 4, 5 | localisation tokens (`RANK_PVT`, `RANK_PVT_FULL`) | | |
| 6 | the name of the insignia art | `rank_pvt1` | `rank_comm1` |
| 7 | experience after the rank | 500 | 2 516 000 |
| 12, 13 | the level number | 1 | 70 |
| 17 | the perk tier the rank unlocks | | |

`maxrank | 69` fixes the number of ranks at **70**; `maxprestige | 10` in the icon table fixes the
prestige ceiling at **10**. A whole career is 2 516 000 experience.

## The insignia are archive art

The art ships only as `.iwi` files in `main/iw_*.iwd`:

- `images/cardicon_rank_<name>.iwi` — 22 files, 64x64 DXT5, no mips (4128 bytes each). The 70 ranks
  share them: ranks 1-3 are `pvt1`, 4-6 `pfc1`, and so on.
- `images/cardicon_prestige_<n>.iwi` — 10 files, the same size, one per prestige level.
  (`cardicon_prestige10anim.iwi` is a 512x256 animation; not used.)

Three of the names the table uses are spelled differently in the art. Measured against the
archives, every other name matches as it stands:

| `mp/rankicontable.csv` | archive |
| --- | --- |
| `rank_sfc1` | `cardicon_rank_sgtfc1.iwi` |
| `rank_1stsgt1` | `cardicon_rank_fsgt1.iwi` |
| `rank_comm1` | `cardicon_rank_comm.iwi` |

`crates/spdesc-mw2` carries that mapping and a test that walks every rank's name to an existing
file, so a misspelling cannot pass unnoticed.

## The cues

| Cue | Alias | Where its audio is |
| --- | --- | --- |
| reaching a rank | `sp_level_up` | *streamed*: `Sound/mp/mp_levelup_final.wav` in the archives |
| prestiging | `enter_prestige` | *loaded* in `code_post_gfx.ff`: mono 44.1 kHz, 3.98 s |

`consider_prestige` (0.22 s) and `exit_prestige` (1.67 s) are the other two prestige cues, and read
as menu navigation rather than a fanfare, which is why the promotion uses `enter_prestige`.

## The rules

`Progression` holds the rank index, the prestige, and the experience inside the rank. `award` is a
pure function of the experience added: it promotes through as many ranks as the award reaches, in
order, and at the last rank collects towards prestige rather than overflowing the rank. Prestige is a
choice — the game asks for it from a menu — so `award` reports `PrestigeReady` once and
`prestige_up` acts on it, resetting the rank and the experience as the game does.

What a target pays is project-owned: [`XP_PER_HIT`](../../crates/spdesc-game-core/src/progression.rs)
is 10 and `XP_PER_KILL` is 100, the latter being Modern Warfare 2's team-deathmatch kill. At those
numbers a career is about 25 000 kills, so the app also takes `P` to grant a chunk of experience and
`O` to take the prestige on offer — a way to walk the whole system without the grind.

## Limits

- **The app shows the level and the insignia, not the localised rank title.** Column 1 is the
  game's internal name (`pvt1`); the display name is the `RANK_PVT` token in column 4, whose text
  lives in the `Localize` assets, which this reader does not capture. The HUD shows the level
  number and the insignia instead.
- **Nothing is saved yet.** A career lives for the run; persistence belongs with
  `spdesc-config/`, beside the character selection.
- **Experience is awarded on hits and kills only.** Modern Warfare 2's other sources — match bonus,
  challenge completions, objective play — are not modelled.
- **The prestige animation art is unused**; the still insignia are what the HUD shows.
