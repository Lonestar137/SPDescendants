# Mechanics evidence boundary

**Status:** observed client metadata; not a behavioral specification. This note
identifies useful investigation seams while protecting the distinction between
what a shipped client names and what the offline game chooses to simulate.

## Scope and provenance

The observations came from read-only metadata available in the target player's
`TFD_DATA`, including `M1-Win64-Shipping.exe` string tables and the readable
global IoStore name/script tables. No original runtime, server, account, or
network flow was launched or required.

The resulting name lists, parsed metadata, executable output, and probes remain
under ignored local state. They are not source code, formulas, or project
content. This document preserves only the high-level evidence useful for
planning an offline, deterministic implementation.

## What the metadata establishes

The observed names provide evidence that the shipped client has separable
systems for:

- a stat vocabulary spanning offensive, defensive, resource, movement, weapon,
  traversal, and skill modifiers;
- modifier operations named Add, Multiply, Max, and Override;
- weapon fire, reload, charge, recoil, ammunition, and hit/damage phases;
- HP and shield damage channels, elemental and delivery classifications, and
  calculator-like damage components;
- movement overrides, jump/sprint/climb concepts, grappling/wire states, and
  projectile or targeting helpers; and
- cooldown, skill, status-effect, and event concepts.

This is enough to choose focused research targets and organize project-owned
types. It is **not** evidence for numeric values, formula order, snapshot
timing, reachability, authority rules, or the original server's behavior.
Names that resemble replication or server calls are especially not a reason to
add networking: SPDescendants remains an offline single-player game.

## Implications for this project

`spdesc-game-core` owns the deterministic local rules. The current combat slice
uses a fire interval, magazine, reload, nearest-hit test, and health damage
because those make a small testable local loop; its defaults are documented
project decisions until a future evidence-backed import is deliberately adopted.

If future work uses this evidence, preserve these boundaries:

| Evidence can guide | Evidence cannot decide |
| --- | --- |
| Names for candidate subsystems and investigation targets | Damage formulas, tuning values, or execution order |
| A deterministic local state-machine shape | Client/server replication or authority behavior |
| Which table or export to validate next | Whether a field name proves a value's meaning |
| Test scenarios that need an explicit local rule | The project's gameplay design choices |

## Required path from evidence to gameplay

For every behavior promoted from an observed name into the game:

1. establish the relevant data or behavior with a read-only source or bounded
   probe;
2. write the project-owned rule and its provenance explicitly;
3. implement it in `spdesc-game-core` without a Bevy, clock, network, or
   external-database dependency;
4. add deterministic tests that inject time and randomness; and
5. keep unresolved details visibly local rather than presenting an inference as
   original-game parity.

The weapon loop is an appropriate narrow example. Broad recovery of every
system, formula, or online feature is not implied by this evidence and remains
outside the current vertical slice.
