//! Driving and presenting the player: input into the simulation's intent types, the simulation
//! step, the entity's transform, and the third-person camera.
//!
//! The app decides nothing here: `spdesc-game-core` owns how a character moves
//! ([`spdesc_game_core::movement`]); this module reads a keyboard and mouse, converts them to a
//! [`MoveIntent`] and a camera orbit, advances the simulation by the frame's delta, and copies the
//! result onto a Bevy [`Transform`]. The camera orbits the character independently of its facing,
//! and movement is relative to the camera, so the character stays put while the camera is moved.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use spdesc_data::anim::AnimClip;
use spdesc_game_core::movement::{Character, MoveIntent, MovementParams};

use crate::content::Skeleton;

/// Degrees of camera yaw/pitch per count of mouse motion.
const MOUSE_SENSITIVITY: f64 = 0.12;
/// The camera's distance behind the character, metres.
const CAMERA_DISTANCE: f32 = 4.0;
/// The camera's look-at height above the character's feet, metres.
const CAMERA_TARGET_HEIGHT: f32 = 1.4;
/// The camera's pitch clamp, degrees.
const MAX_PITCH: f64 = 80.0;

/// The player entity. The simulation state lives here; presentation reads it.
#[derive(Component, Default)]
pub struct Player {
    pub character: Character,
    pub params: MovementParams,
}

/// The camera's orbit around the player, independent of the player's facing.
#[derive(Component, Default)]
pub struct CameraRig {
    /// Orbit yaw, degrees counter-clockwise from +X (the game frame).
    pub yaw: f64,
    /// Orbit pitch, degrees up.
    pub pitch: f64,
}

/// A bone with a small procedural idle: it oscillates about its bind rotation so the character is
/// not frozen in its bind pose. Game-accurate animation (the shipped `AnimSequence` codec) is a
/// later slice; this is the documented local stand-in.
#[derive(Component)]
pub struct Animator {
    pub clips: Vec<AnimClip>,
    pub idle: usize,
    pub walk: usize,
    pub run: usize,
    pub bone_to_track: Vec<Vec<Option<usize>>>,
    pub bones: Vec<Entity>,
    pub skeleton: Skeleton,
    pub time: f32,
}

/// Pose the skeleton from the active locomotion clip. Standing still plays the idle, moving plays
/// the walk, and sprinting the run; the clip is looped and sampled by time.
pub fn animate_locomotion(
    time: Res<Time>,
    mut players: Query<(&Player, &mut Animator)>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    for (player, mut animator) in &mut players {
        let speed = {
            let v = player.character.velocity;
            v[0].hypot(v[1])
        };
        let index = if speed > 600.0 {
            animator.run
        } else if speed > 20.0 {
            animator.walk
        } else {
            animator.idle
        };
        if index >= animator.clips.len() {
            continue;
        }
        let (num_samples, sample_rate) = {
            let clip = &animator.clips[index];
            (clip.num_samples, clip.sample_rate)
        };
        let duration = num_samples as f32 / sample_rate;
        if duration <= 0.0 {
            continue;
        }
        animator.time = (animator.time + dt) % duration;
        let sample = ((animator.time * sample_rate) as usize).min(num_samples as usize - 1);
        let locals = crate::content::pose_bone_locals(
            &animator.skeleton,
            &animator.clips[index],
            &animator.bone_to_track[index],
            sample,
        );
        for (entity, local) in animator.bones.iter().zip(locals.iter()) {
            if let Ok(mut transform) = transforms.get_mut(*entity) {
                *transform = *local;
            }
        }
    }
}

/// The character mesh is authored facing the game's `+Y` (its right axis) rather than `+X` (the
/// gameplay forward), so a fixed quarter turn aligns the model with the direction it moves. This
/// is a presentation offset for this asset, applied to the entity's rotation only.
const MODEL_YAW_OFFSET: f32 = -std::f32::consts::FRAC_PI_2;

/// Game-space position (cm, `+Z` up) to Bevy's (m, `+Y` up): `(x, y, z)` → `(x, z, -y) * 0.01`.
fn position_to_bevy(p: [f64; 3]) -> Vec3 {
    Vec3::new(
        (p[0] * 0.01) as f32,
        (p[2] * 0.01) as f32,
        (-p[1] * 0.01) as f32,
    )
}

/// A camera look direction (game yaw/pitch, degrees) to Bevy's frame.
fn look_direction_to_bevy(yaw: f64, pitch: f64) -> Vec3 {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    Vec3::new(
        (p.cos() * y.cos()) as f32,
        p.sin() as f32,
        (-p.cos() * y.sin()) as f32,
    )
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

/// Read input, orbit the camera with the mouse, advance the player's simulation by the frame
/// delta, and write the result to the player's transform.
pub fn drive_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<AccumulatedMouseMotion>,
    windows: Query<&CursorOptions, With<PrimaryWindow>>,
    mut rigs: Query<&mut CameraRig>,
    mut players: Query<(&mut Player, &mut Transform)>,
) {
    // Cap the step so a hitch cannot teleport the character or blow up the integration.
    let dt = (time.delta_secs() as f64).clamp(0.0, 0.05);

    // Mouse-look only while the cursor is grabbed; a windowed (headless) app defaults to looking.
    let grabbed = windows
        .single()
        .map(|o| o.grab_mode != CursorGrabMode::None)
        .unwrap_or(true);
    if grabbed {
        for mut rig in &mut rigs {
            rig.yaw = (rig.yaw + f64::from(mouse.delta.x) * MOUSE_SENSITIVITY).rem_euclid(360.0);
            rig.pitch = (rig.pitch - f64::from(mouse.delta.y) * MOUSE_SENSITIVITY)
                .clamp(-MAX_PITCH, MAX_PITCH);
        }
    }
    let frame_yaw = rigs.single().map(|r| r.yaw).unwrap_or(0.0);

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

    for (mut player, mut transform) in &mut players {
        let params = player.params;
        player.character.step(&params, intent, frame_yaw, 0.0, dt);
        transform.translation = position_to_bevy(player.character.position);
        transform.rotation =
            Quat::from_rotation_y(player.character.yaw.to_radians() as f32 + MODEL_YAW_OFFSET);
    }
}

/// Place the camera on its orbit around the player, aimed at its upper body. The orbit is
/// independent of the player's facing, so the player can be viewed from any side.
pub fn follow_camera(
    players: Query<&Player>,
    rigs: Query<&CameraRig>,
    mut cameras: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
) {
    let (Ok(player), Ok(rig), Ok(mut camera)) =
        (players.single(), rigs.single(), cameras.single_mut())
    else {
        return;
    };
    let dir = look_direction_to_bevy(rig.yaw, rig.pitch);
    let target = position_to_bevy(player.character.position) + Vec3::Y * CAMERA_TARGET_HEIGHT;
    let eye = target - dir * CAMERA_DISTANCE;
    *camera = Transform::from_translation(eye).looking_at(target, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    /// The wiring this module owns: movement follows the input relative to the camera, and the
    /// mouse orbits the camera without turning the character. Proven headless.
    #[test]
    fn movement_follows_the_camera_and_the_mouse_orbits_it() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<AccumulatedMouseMotion>();
        let mut time = Time::<()>::default();
        time.advance_by(Duration::from_millis(100));
        app.insert_resource(time);
        app.add_systems(Update, drive_player);

        let player = app
            .world_mut()
            .spawn((Player::default(), Transform::default()))
            .id();
        let camera = app.world_mut().spawn(CameraRig::default()).id();

        // Forward input with the camera at yaw 0: the character moves along Bevy +X (the step is
        // clamped to 0.05 s, so 400 cm/s * 0.05 s = 0.2 m).
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.update();
        let transform = *app.world().entity(player).get::<Transform>().unwrap();
        assert!(
            (transform.translation.x - 0.2).abs() < 1e-4,
            "expected a step along +X, got {:?}",
            transform.translation
        );

        // Standing still, turning the mouse right orbits the camera without turning the character.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyW);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(100.0, 0.0);
        app.update();
        let rig = app.world().entity(camera).get::<CameraRig>().unwrap();
        assert!(rig.yaw > 1.0, "the camera must orbit with the mouse");
        let character_yaw = app
            .world()
            .entity(player)
            .get::<Player>()
            .unwrap()
            .character
            .yaw;
        assert_eq!(
            character_yaw, 0.0,
            "the character does not turn with the camera"
        );

        // Now forward input goes the way the camera faces (turned), not +X.
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::ZERO;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.update();
        let transform = *app.world().entity(player).get::<Transform>().unwrap();
        assert!(
            transform.translation.z < -0.01,
            "movement must follow the camera yaw, got {:?}",
            transform.translation
        );
    }
}
