//! `spdesc` — the SPDescendants client.
//!
//! Presentation only: the simulation lives in `spdesc-game-core` and is Bevy-free. The app
//! resolves the read-only `GAME_DATA` folder through `spdesc_game_core::game_data` and never
//! writes into it.

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "SPDescendants".into(),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, (announce_game_data, spawn_camera))
        .run();
}

/// Report where the resolver landed, or where it looked: the first frame of evidence that the
/// `GAME_DATA` rule is wired correctly on this machine.
fn announce_game_data() {
    match spdesc_game_core::game_data() {
        Some(data) => info!("game data: {}", data.display()),
        None => {
            warn!("no game data found. Looked in, in order:");
            for c in spdesc_game_core::candidates() {
                warn!("  {}", c.display());
            }
            warn!("set GAME_DATA=<folder> (empty means 'no data' on purpose)");
        }
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}
