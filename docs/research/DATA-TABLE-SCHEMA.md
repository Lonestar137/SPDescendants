# Data-table schema evidence

**Status:** local research observation, not yet a production table-reading
contract. This note preserves the boundary between useful schema evidence and
unsupported guesses so future gameplay work does not treat a local mapping as
authority.

## Scope and provenance

Cooked unversioned properties require an ordered property schema. The runtime
loads a player-supplied mapping from the ignored local-state root; see
`crates/spdesc-data/src/mapping.rs`. That mapping, any `.usmap`, generated JSON,
SDK dump, table extract, row bytes, and downloaded third-party source are not
committed.

Read-only experiments used the target build's data-table packages plus a public
reflection-header candidate. The candidate is evidence only: it is neither a
build-matching mapping nor a license to vendor external headers. The upstream
candidate was inspected at
[`slemknot-lab/TFD-SDK` commit `2e512c2`](https://github.com/slemknot-lab/TFD-SDK/tree/2e512c2b8f37e45a2371610e34574d9825f14aba).

## Validated and provisional findings

| Subject | Evidence status | Durable conclusion |
| --- | --- | --- |
| Newer downloadable mapping | Observed search result | No build-validated newer `.usmap` was found during the bounded search. This is not proof that one does not exist. |
| Ranged weapons | Observed local probe | The candidate schema consumed all 145 observed rows to their individual export boundaries. |
| Melee weapons | Observed local probe | The candidate schema consumed all 4 observed rows to their individual export boundaries. |
| Ranged-weapon recoil | Observed local probe | The candidate schema consumed all 215 observed rows to their individual export boundaries. |
| Skills | Observed local failure | The candidate completed only the first observed row; it then lost name/cursor alignment. It must not be used as a skill schema. |

For the passing tables, independently observed anchors agreed with the candidate
for the early weapon fields: reload/rounds-class references, fire-mode-related
fields, recoil references, the row name, blueprint path, template identifier,
and localized-string key. This establishes useful framing and name-resolution
consistency, **not** every field's semantic meaning or a complete build schema.

## Important correction to earlier inference

An earlier wire-only interpretation treated the ranged-weapon head as a
misordered or partially unknown schema. The later inheritance-aware candidate
accounts for that head through this property chain:

```text
M1RangedWeaponData -> M1WeaponData -> M1EquipmentData -> TableRowBase
```

That is a stronger explanation for the tested weapon rows. It supersedes the
older claim that the observed leading fields were necessarily an unresolved
insertion or reorder. It does not make the candidate authoritative for skills,
other tables, or another game build.

## Project boundary

There is currently no production data-table reader. The default weapon values
in `crates/spdesc-game-core/src/combat.rs` are explicitly project-owned local
rules, not values extracted from a table. No gameplay implementation may adopt
a candidate field, formula, or numeric value merely because a schema appears to
frame a row.

Before a table drives project behavior, require all of the following:

1. a read-only source citation for the target package and build;
2. an independently validated property ordering and native-struct treatment;
3. exact-boundary parsing across a representative table set;
4. cross-table or behavioral validation for each field used by gameplay; and
5. data-gated tests plus an explicit project-owned rule for anything that
   remains unknown.

Do not silently fall back to a shorter schema, skip unread bytes, or promote a
third-party dump into a checked-in mapping. A failure to frame a row is evidence
that the input is unsupported, not a request to guess.
