//! Driving and presenting the player: input into the simulation's intent types, the simulation
//! step, the entity's transform, and the third-person camera.
//!
//! The app decides nothing here: `spdesc-game-core` owns how a character moves
//! ([`spdesc_game_core::movement`]); this module only reads a keyboard and mouse, converts them to
//! [`MoveIntent`]/[`LookIntent`], advances the simulation by the frame's delta, and copies the
//! result onto a Bevy [`Transform`].

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use spdesc_game_core::movement::{Character, LookIntent, MoveIntent, MovementParams};

/// Degrees of yaw/pitch per count of mouse motion.
const MOUSE_SENSITIVITY: f64 = 0.12;
/// The camera's distance behind the character, metres.
const CAMERA_DISTANCE: f32 = 4.0;
/// The camera's look-at height above the character's feet, metres.
const CAMERA_TARGET_HEIGHT: f32 = 1.4;

/// The player entity. The simulation state lives here; presentation reads it.
#[derive(Component, Default)]
pub struct Player {
    pub character: Character,
    pub params: MovementParams,
}

/// Game-space position (cm, `+Z` up) to Bevy's (m, `+Y` up): `(x, y, z)` → `(x, z, -y) * 0.01`.
fn position_to_bevy(p: [f64; 3]) -> Vec3 {
    Vec3::new(
        (p[0] * 0.01) as f32,
        (p[2] * 0.01) as f32,
        (-p[1] * 0.01) as f32,
    )
}

/// Game-space direction (unit) to Bevy's.
fn direction_to_bevy(d: [f64; 3]) -> Vec3 {
    Vec3::new(d[0] as f32, d[2] as f32, -d[1] as f32)
}

/// Grab and hide the cursor on startup, so mouse-look works without the pointer leaving the
/// window.
pub fn grab_cursor(mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    for mut options in &mut windows {
        options.grab_mode = CursorGrabMode::Locked;
        options.visible = false;
    }
}

/// Release the cursor on Escape (and grab it again on the next press), so the window is escapable
/// without quitting.
pub fn toggle_cursor(
    keys: Res<ButtonInput<KeyCode>>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    for mut options in &mut windows {
        let grabbed = options.grab_mode != CursorGrabMode::None;
        options.grab_mode = if grabbed {
            CursorGrabMode::None
        } else {
            CursorGrabMode::Locked
        };
        options.visible = grabbed;
    }
}

/// Read input, advance the player's simulation by the frame delta, and write the result to the
/// player's transform.
pub fn drive_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<AccumulatedMouseMotion>,
    windows: Query<&CursorOptions, With<PrimaryWindow>>,
    mut players: Query<(&mut Player, &mut Transform)>,
) {
    // Cap the step so a hitch cannot teleport the character or blow up the integration.
    let dt = (time.delta_secs() as f64).clamp(0.0, 0.05);

    let mut intent = MoveIntent {
        sprint: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        jump: keys.just_pressed(KeyCode::Space),
        ..default()
    };
    if keys.pressed(KeyCode::KeyW) {
        intent.forward += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        intent.forward -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        intent.right += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        intent.right -= 1.0;
    }
    // Mouse-look only while the cursor is grabbed; a windowed (headless) app defaults to looking.
    let grabbed = windows
        .single()
        .map(|o| o.grab_mode != CursorGrabMode::None)
        .unwrap_or(true);
    // Window/`AccumulatedMouseMotion`: `+x` is right, `+y` is down. Right turns the yaw up; up
    // (negative `y`) raises the pitch.
    let look = if grabbed {
        LookIntent {
            yaw_delta: f64::from(mouse.delta.x) * MOUSE_SENSITIVITY,
            pitch_delta: -f64::from(mouse.delta.y) * MOUSE_SENSITIVITY,
        }
    } else {
        LookIntent::default()
    };

    for (mut player, mut transform) in &mut players {
        let params = player.params;
        player.character.step(&params, intent, look, 0.0, dt);
        transform.translation = position_to_bevy(player.character.position);
        transform.rotation = Quat::from_rotation_y(player.character.yaw.to_radians() as f32);
    }
}

/// Place the camera behind the player along its look direction, aimed at its upper body.
pub fn follow_camera(
    players: Query<&Player>,
    mut cameras: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
) {
    let Ok(player) = players.single() else {
        return;
    };
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    let dir = direction_to_bevy(player.character.look_direction());
    let target = position_to_bevy(player.character.position) + Vec3::Y * CAMERA_TARGET_HEIGHT;
    let eye = target - dir * CAMERA_DISTANCE;
    *camera = Transform::from_translation(eye).looking_at(target, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    /// The wiring this module owns — a held key and a mouse delta become a simulation step and a
    /// moved transform — proven headless, without a window or a GPU.
    #[test]
    fn forward_input_moves_the_player_and_mouse_turns_it() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<AccumulatedMouseMotion>();
        // A fixed step: without TimePlugin the resource keeps the delta we set.
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_millis(100));
        app.insert_resource(time);
        app.add_systems(Update, drive_player);

        let player = app
            .world_mut()
            .spawn((Player::default(), Transform::default()))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.update();

        let transform = *app.world().entity(player).get::<Transform>().unwrap();
        // Yaw 0 faces +X; the step is clamped to 0.05 s, so 400 cm/s * 0.05 s = 0.2 m along +X.
        assert!(
            (transform.translation.x - 0.2).abs() < 1e-4,
            "expected a step along +X, got {:?}",
            transform.translation
        );

        // Turning the mouse right increases yaw, so a second step veers toward Bevy -Z.
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(100.0, 0.0);
        app.update();
        let transform = *app.world().entity(player).get::<Transform>().unwrap();
        assert!(
            transform.translation.z < -0.01,
            "turning right must change z, got {:?}",
            transform.translation
        );
    }
}
