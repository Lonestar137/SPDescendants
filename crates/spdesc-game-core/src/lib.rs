//! The Bevy-free game simulation: rules, converted content, and persistence.
//!
//! Nothing in this crate depends on Bevy or on presentation types, so the simulation is testable
//! headless and deterministic: time and randomness are injected, never read from the host clock.
//!
//! [`data`] is this workspace's one resolver for each external data folder — `TFD_DATA`, the
//! game's own assets, and `MW2_DATA`, the optional sound source. Both are read-only runtime input:
//! never written, never committed.

pub mod combat;
pub mod data;
pub mod movement;
pub mod progression;

// The `tfd_data_or_skip!` and `mw2_data_or_skip!` macros are `#[macro_export]`, so they are
// already re-exported at the crate root: `spdesc_game_core::tfd_data_or_skip!(...)`.
pub use data::{ALL, DataRoot, MW2, TFD, mw2_data, skipped, tfd_data};
pub use progression::{Progression, Promotion, Rank, Ranks, XP_PER_HIT, XP_PER_KILL};
