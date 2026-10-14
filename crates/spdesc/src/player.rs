//! Driving and presenting the player: input into the simulation's intent types, the simulation
//! step, the entity's transform, and the third-person camera.
//!
//! The app decides nothing here: `spdesc-game-core` owns how a character moves
//! ([`spdesc_game_core::movement`]); this module reads a keyboard and mouse, converts them to a
//! [`MoveIntent`] and a camera orbit, advances the simulation by the frame's delta, and copies the
//! result onto a Bevy [`Transform`]. Movement is camera-relative: forward is the way the camera
//! faces, and the character turns to follow it, so the camera and the character move together.

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

/// Poses the character's skeleton each frame from the decoded clips: idle/walk/run on the ground, a
/// takeoff clip then an airborne loop while in the air, and a landing clip on touchdown.
#[derive(Component)]
pub struct Animator {
    pub clips: Vec<AnimClip>,
    pub idle: usize,
    pub walk: usize,
    pub run: usize,
    pub jump: usize,
    pub air: usize,
    pub land: usize,
    pub bone_to_track: Vec<Vec<Option<usize>>>,
    pub bones: Vec<Entity>,
    pub skeleton: Skeleton,
    /// Looping locomotion time, seconds.
    pub time: f32,
    /// Seconds since the jump took off.
    pub air_timer: f32,
    /// Seconds left of the landing one-shot.
    pub land_timer: f32,
    /// Whether the character was on the ground last frame (to detect a touchdown).
    pub was_grounded: bool,
}

fn clip_duration(clip: &AnimClip) -> f32 {
    if clip.sample_rate <= 0.0 {
        0.0
    } else {
        clip.num_samples as f32 / clip.sample_rate
    }
}

/// Which clip family the character is playing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Grounded locomotion (idle/walk/run).
    Local,
    /// Airborne takeoff.
    Jump,
    /// Airborne loop.
    Air,
    /// Touchdown.
    Land,
}

/// The animation phase from the character's grounded state and timers. `Land` requires being
/// grounded, so a landing pose never plays in the air (only on touchdown).
fn phase(grounded: bool, air_timer: f32, land_timer: f32, jump_duration: f32) -> Phase {
    if !grounded {
        if air_timer < jump_duration {
            Phase::Jump
        } else {
            Phase::Air
        }
    } else if land_timer > 0.0 {
        Phase::Land
    } else {
        Phase::Local
    }
}

/// Advance and apply the character animation. Grounded: idle/walk/run, looped. Airborne: the
/// takeoff clip once, then the airborne loop. On touchdown: the landing clip once, then back to
/// locomotion. Nothing plays a landing pose until the character is actually on the ground.
pub fn animate_locomotion(
    time: Res<Time>,
    mut players: Query<(&Player, &mut Animator)>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    for (player, mut animator) in &mut players {
        let grounded = player.character.grounded;
        let velocity = player.character.velocity;
        let speed = velocity[0].hypot(velocity[1]);

        let jump_duration = clip_duration(&animator.clips[animator.jump]);
        let air_duration = clip_duration(&animator.clips[animator.air]);
        let land_duration = clip_duration(&animator.clips[animator.land]);

        if grounded {
            if !animator.was_grounded {
                animator.land_timer = land_duration;
            }
            animator.air_timer = 0.0;
        } else {
            animator.air_timer += dt;
        }
        animator.was_grounded = grounded;

        // Choose the clip and how far into it (seconds) we are.
        let (index, at, looping) = match phase(
            grounded,
            animator.air_timer,
            animator.land_timer,
            jump_duration,
        ) {
            Phase::Jump => (animator.jump, animator.air_timer, false),
            Phase::Air => {
                let at = (animator.air_timer - jump_duration) % air_duration.max(1e-3);
                (animator.air, at, true)
            }
            Phase::Land => {
                let at = land_duration - animator.land_timer;
                animator.land_timer = (animator.land_timer - dt).max(0.0);
                (animator.land, at, false)
            }
            Phase::Local => {
                let index = if speed > 600.0 {
                    animator.run
                } else if speed > 20.0 {
                    animator.walk
                } else {
                    animator.idle
                };
                let duration = clip_duration(&animator.clips[index]);
                animator.time = (animator.time + dt) % duration.max(1e-3);
                (index, animator.time, true)
            }
        };

        let clip = &animator.clips[index];
        // The takeoff and landing clips are one-shots: hold the last frame if the phase outlasts
        // them (a long jump holds the takeoff; the landing always plays to its end).
        let at = if looping {
            at
        } else {
            at.min(clip_duration(clip))
        };
        let sample = ((at * clip.sample_rate) as usize).min(clip.num_samples as usize - 1);
        let locals = crate::content::pose_bone_locals(
            &animator.skeleton,
            clip,
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
    let down = |a: KeyCode, b: KeyCode| keys.pressed(a) || keys.pressed(b);
    if down(KeyCode::KeyW, KeyCode::ArrowUp) {
        intent.forward += 1.0;
    }
    if down(KeyCode::KeyS, KeyCode::ArrowDown) {
        intent.forward -= 1.0;
    }
    if down(KeyCode::KeyD, KeyCode::ArrowRight) {
        intent.right += 1.0;
    }
    if down(KeyCode::KeyA, KeyCode::ArrowLeft) {
        intent.right -= 1.0;
    }

    for (mut player, mut transform) in &mut players {
        let params = player.params;
        // Camera-relative movement: forward is the way the camera faces, so the character turns to
        // follow the camera and the sticks always steer relative to the view.
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

    /// The animator never plays a landing pose in the air: on the ground it is idle/walk/run, or a
    /// landing once; airborne it is takeoff then the airborne loop.
    #[test]
    fn landing_phase_only_happens_on_the_ground() {
        assert_eq!(
            phase(false, 0.1, 0.0, 1.0),
            Phase::Jump,
            "takeoff while rising"
        );
        assert_eq!(
            phase(false, 1.5, 0.0, 1.0),
            Phase::Air,
            "air loop after takeoff"
        );
        assert_eq!(
            phase(false, 1.5, 1.0, 1.0),
            Phase::Air,
            "a landing timer must be ignored in the air"
        );
        assert_eq!(phase(true, 0.0, 1.0, 1.0), Phase::Land, "land on touchdown");
        assert_eq!(
            phase(true, 0.0, 0.0, 1.0),
            Phase::Local,
            "locomotion on the ground"
        );
    }

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

        // Camera-relative: forward now follows the orbited camera, so the step gains a -Z (game
        // +Y) component instead of staying on +X.
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
            "camera-relative movement must follow the camera yaw, got {:?}",
            transform.translation
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyW);

        // "Right" moves the character to the player's right: at yaw 0 the game's left-handed frame
        // puts it along -Y (what the forward-facing camera shows on screen as right).
        if let Some(mut rig) = app.world_mut().get_mut::<CameraRig>(camera) {
            *rig = CameraRig::default();
        }
        let before = app
            .world()
            .entity(player)
            .get::<Player>()
            .unwrap()
            .character
            .position;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowRight);
        app.update();
        let after = app
            .world()
            .entity(player)
            .get::<Player>()
            .unwrap()
            .character
            .position;
        assert!(
            after[1] < before[1] - 1.0 && (after[0] - before[0]).abs() < 1e-4,
            "right must move toward -Y at yaw 0, got {before:?} -> {after:?}"
        );
    }
}
