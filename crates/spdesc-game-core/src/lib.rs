//! The Bevy-free game simulation: rules, converted content, and persistence.
//!
//! Nothing in this crate depends on Bevy or on presentation types, so the simulation is testable
//! headless and deterministic: time and randomness are injected, never read from the host clock.
//!
//! [`game_data`] is this workspace's one resolver for the `GAME_DATA` folder (read-only runtime
//! input — never written, never committed).

pub mod game_data;

// The `game_data_or_skip!` macro is `#[macro_export]`, so it is already re-exported at the crate
// root: `spdesc_game_core::game_data_or_skip!(...)`.
pub use game_data::{candidates, game_data, skipped};
