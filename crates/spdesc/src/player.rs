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

use crate::combat::Weapon;
use crate::content::{Locomotion, Reload, Skeleton};

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
    /// Whether the right mouse button is held: aim down sights.
    pub aiming: bool,
}

/// The camera's orbit around the player, independent of the player's facing.
#[derive(Component, Default)]
pub struct CameraRig {
    /// Orbit yaw, degrees counter-clockwise from +X (the game frame).
    pub yaw: f64,
    /// Orbit pitch, degrees up.
    pub pitch: f64,
    /// Aim-down-sights blend, 0 (hip) to 1 (over the shoulder and zoomed).
    pub ads: f32,
}

/// Poses the character's skeleton each frame from the decoded clips: idle/walk/run on the ground, a
/// takeoff clip then an airborne loop while in the air, and a landing clip on touchdown. Changing
/// clip cross-fades rather than snapping, so speeding up, slowing to a stop, and jumping all ease.
#[derive(Component)]
pub struct Animator {
    pub clips: Vec<AnimClip>,
    pub bone_to_track: Vec<Vec<Option<usize>>>,
    /// The unarmed locomotion set, and the armed (two-handed) one used while carrying a weapon.
    pub base: Locomotion,
    pub armed: Option<Locomotion>,
    /// The aim-down-sights pose, and the weapon reload's halves.
    pub aim: usize,
    pub reload: Option<Reload>,
    /// Which bones the aim and reload poses override (the spine from `Spine1` up, arms and head).
    pub upper: Vec<bool>,
    /// The bone the upper-body layer pivots at, and the weapon's socket on the hand. Aiming rotates
    /// the upper body so the socket's `+X` (the barrel) lands on the aim direction.
    pub upper_root: usize,
    pub weapon_socket: Option<usize>,
    /// The eased weight of the aim layer, and its looping time through the aim clip.
    pub ads: f32,
    pub aim_time: f32,
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
    /// The clip playing this frame, how far into it, and whether it loops.
    pub active: usize,
    pub active_at: f32,
    pub active_looping: bool,
    /// The clip being cross-faded out of, frozen at its last time.
    pub prev: usize,
    pub prev_at: f32,
    pub prev_looping: bool,
    /// Cross-fade progress toward `active` (1.0 = settled).
    pub blend: f32,
}

/// Seconds a clip change takes to cross-fade. TFD has no dedicated "stop" clip in the `MoveF_0LT`
/// set (the BlendSpace blends speed down to the idle), so e.g. running to a stop is this fade.
const BLEND_SECONDS: f32 = 0.18;
/// Seconds the aim pose takes to raise into and back out of the upper body.
const ADS_SECONDS: f32 = 0.14;
/// The bone the aim layer starts at; it and its descendants take the aim pose.
const UPPER_BODY_ROOT: &str = "Bip001-Spine1";
/// How far the upper body may pitch to follow the camera, radians.
const AIM_LEAN_MAX: f32 = 0.6;
/// The most the upper body may turn to bring the barrel onto the aim line, radians.
const AIM_TURN_MAX: f32 = 1.0;

/// The index of the bone the aim layer pivots at (`Bip001-Spine1`), or 0.
pub fn upper_body_root(skeleton: &Skeleton) -> usize {
    skeleton
        .names
        .iter()
        .position(|name| name == UPPER_BODY_ROOT)
        .unwrap_or(0)
}

/// The bones the aim pose overrides: `UPPER_BODY_ROOT` and every descendant (neck, head,
/// clavicles, arms, hands, fingers). The pelvis and legs keep the locomotion pose, so the character
/// still runs and jumps while aiming.
pub fn upper_body_mask(skeleton: &Skeleton) -> Vec<bool> {
    let n = skeleton.names.len();
    let mut upper = vec![false; n];
    let Some(root) = skeleton
        .names
        .iter()
        .position(|name| name == UPPER_BODY_ROOT)
    else {
        return upper;
    };
    upper[root] = true;
    for i in 0..n {
        let parent = skeleton.parents[i];
        if parent >= 0 && upper[parent as usize] {
            upper[i] = true;
        }
    }
    upper
}

/// Smoothstep, so a cross-fade eases in and out rather than moving linearly.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn clip_duration(clip: &AnimClip) -> f32 {
    if clip.sample_rate <= 0.0 {
        0.0
    } else {
        clip.num_samples as f32 / clip.sample_rate
    }
}

/// The sample index of `clip` at time `at`, clamping a one-shot to its last frame.
fn sample_at(clip: &AnimClip, at: f32, looping: bool) -> usize {
    let at = if looping {
        at
    } else {
        at.min(clip_duration(clip))
    };
    ((at * clip.sample_rate) as usize).min(clip.num_samples as usize - 1)
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
    rigs: Query<&CameraRig>,
    mut players: Query<(&Player, &mut Animator, Option<&Weapon>)>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    let aim_pitch = rigs.single().map(|rig| rig.pitch as f32).unwrap_or(0.0);
    for (player, mut animator, weapon) in &mut players {
        let grounded = player.character.grounded;
        let velocity = player.character.velocity;
        let speed = velocity[0].hypot(velocity[1]);

        // The armed stance replaces the unarmed locomotion wholesale while carrying a weapon.
        let loco = animator.armed.unwrap_or(animator.base);
        let jump_duration = clip_duration(&animator.clips[loco.jump]);
        let air_duration = clip_duration(&animator.clips[loco.air]);
        let land_duration = clip_duration(&animator.clips[loco.land]);

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
            Phase::Jump => (loco.jump, animator.air_timer, false),
            Phase::Air => {
                let at = (animator.air_timer - jump_duration) % air_duration.max(1e-3);
                (loco.air, at, true)
            }
            Phase::Land => {
                let at = land_duration - animator.land_timer;
                animator.land_timer = (animator.land_timer - dt).max(0.0);
                (loco.land, at, false)
            }
            Phase::Local => {
                let index = if speed > 600.0 {
                    loco.run
                } else if speed > 20.0 {
                    loco.walk
                } else {
                    loco.idle
                };
                let duration = clip_duration(&animator.clips[index]);
                animator.time = (animator.time + dt) % duration.max(1e-3);
                (index, animator.time, true)
            }
        };

        // Start a cross-fade when the chosen clip changes.
        if index != animator.active {
            animator.prev = animator.active;
            animator.prev_at = animator.active_at;
            animator.prev_looping = animator.active_looping;
            animator.blend = 0.0;
        }
        animator.active = index;
        animator.active_at = at;
        animator.active_looping = looping;

        let clip = &animator.clips[index];
        let sample = sample_at(clip, at, looping);
        let mut locals = crate::content::pose_bone_locals(
            &animator.skeleton,
            clip,
            &animator.bone_to_track[index],
            sample,
        );

        // Cross-fade out of the outgoing clip, so e.g. running to a stop eases instead of snapping.
        if animator.blend < 1.0 {
            animator.blend = (animator.blend + dt / BLEND_SECONDS).min(1.0);
            let weight = smoothstep(animator.blend);
            let outgoing_clip = &animator.clips[animator.prev];
            let outgoing_sample = sample_at(outgoing_clip, animator.prev_at, animator.prev_looping);
            let outgoing = crate::content::pose_bone_locals(
                &animator.skeleton,
                outgoing_clip,
                &animator.bone_to_track[animator.prev],
                outgoing_sample,
            );
            for (current, from) in locals.iter_mut().zip(outgoing.iter()) {
                current.translation = from.translation.lerp(current.translation, weight);
                current.rotation = from.rotation.slerp(current.rotation, weight);
                current.scale = from.scale.lerp(current.scale, weight);
            }
        }

        // Layer the aim-down-sights pose over the upper body, eased in and out, while the lower
        // body keeps running the locomotion. Only the upper-body locals are replaced.
        animator.ads = if player.aiming {
            (animator.ads + dt / ADS_SECONDS).min(1.0)
        } else {
            (animator.ads - dt / ADS_SECONDS).max(0.0)
        };
        if animator.ads > 0.0 {
            let duration = clip_duration(&animator.clips[animator.aim]);
            animator.aim_time = (animator.aim_time + dt) % duration.max(1e-3);
            let aim_locals = {
                let aim = &animator.clips[animator.aim];
                let sample = sample_at(aim, animator.aim_time, true);
                crate::content::pose_bone_locals(
                    &animator.skeleton,
                    aim,
                    &animator.bone_to_track[animator.aim],
                    sample,
                )
            };
            let weight = smoothstep(animator.ads);
            for (local, aim_local) in locals
                .iter_mut()
                .zip(aim_locals.iter())
                .zip(&animator.upper)
                .filter(|(_, upper)| **upper)
                .map(|((local, aim_local), _)| (local, aim_local))
            {
                local.translation = local.translation.lerp(aim_local.translation, weight);
                local.rotation = local.rotation.slerp(aim_local.rotation, weight);
                local.scale = local.scale.lerp(aim_local.scale, weight);
            }
            // The character faces the camera's yaw while aiming, so its own yaw plus the camera's
            // pitch is the aim direction.
            let aim_dir = look_direction_to_bevy(player.character.yaw, aim_pitch as f64);
            let entity_yaw = player.character.yaw.to_radians() as f32 + MODEL_YAW_OFFSET;
            if !aim_weapon(&animator, &mut locals, aim_dir, entity_yaw, weight) {
                // Without a weapon to align, at least pitch the torso to follow the camera.
                lean_upper_body(&animator, &mut locals, aim_pitch * weight);
            }
        }

        // A reload takes the upper body. The lower body only takes the reload's own stance while
        // standing: the game plays the reload *over* the locomotion, so walking and running keep
        // their leg cycle and only the hands work the weapon.
        // The weapon owns the timer, so the pose tracks the reload rather than restarting it.
        let reload_left = weapon.map_or(0.0, |weapon| weapon.combat.reload_remaining as f32);
        if let Some(reload) = animator.reload
            && reload_left > 0.0
        {
            let total = clip_duration(&animator.clips[reload.upper]);
            let at = (total - reload_left).max(0.0);
            let standing = speed <= 20.0;
            for (index, upper) in [(reload.upper, true), (reload.lower, false)] {
                if !upper && !standing {
                    continue;
                }
                let clip = &animator.clips[index];
                let sample = sample_at(clip, at, false);
                let posed = crate::content::pose_bone_locals(
                    &animator.skeleton,
                    clip,
                    &animator.bone_to_track[index],
                    sample,
                );
                for ((local, posed), is_upper) in
                    locals.iter_mut().zip(posed.iter()).zip(&animator.upper)
                {
                    if *is_upper == upper {
                        *local = *posed;
                    }
                }
            }
        }

        for (entity, local) in animator.bones.iter().zip(locals.iter()) {
            if let Ok(mut transform) = transforms.get_mut(*entity) {
                *transform = *local;
            }
        }
    }
}

/// The model-space rotation of `bone` from the sampled locals, composing up its parent chain.
fn model_world_rot(animator: &Animator, locals: &[Transform], bone: usize) -> Quat {
    let mut chain = Vec::new();
    let mut bone = bone as i32;
    while bone >= 0 {
        chain.push(bone as usize);
        bone = animator.skeleton.parents[bone as usize];
    }
    let mut world = Quat::IDENTITY;
    for &b in chain.iter().rev() {
        world *= locals[b].rotation;
    }
    world
}

/// Turn the whole upper body so the weapon's barrel (the socket's `+X`) points along `aim_dir`.
///
/// The aim pose alone holds the rifle *at the ready* — authored for the weapon, but its barrel sits
/// well off the sight line (the game adds an aim-offset blend on top, which this MVP does not
/// decode). Rotating the upper body onto the barrel closes that, so the rifle and the reticle
/// agree. Returns false when the character has no weapon to align.
fn aim_weapon(
    animator: &Animator,
    locals: &mut [Transform],
    aim_dir: Vec3,
    entity_yaw: f32,
    weight: f32,
) -> bool {
    let Some(socket) = animator.weapon_socket else {
        return false;
    };
    let root = animator.upper_root;
    if socket >= locals.len() || root >= locals.len() {
        return false;
    }
    let barrel = model_world_rot(animator, locals, socket) * Vec3::X;
    // The aim direction in the model's own frame: the bones hang off the model root, whose rotation
    // (the character's yaw plus the model's quarter turn) the caller supplies.
    let model_aim = Quat::from_rotation_y(entity_yaw).inverse() * aim_dir;
    let turn = Quat::IDENTITY.slerp(clamped_arc(barrel, model_aim), weight.clamp(0.0, 1.0));
    if turn.angle_between(Quat::IDENTITY) < 1e-4 {
        return true;
    }
    // Re-express the model-space turn in the root bone's parent frame, then prepend it.
    let parent = animator.skeleton.parents[root];
    let parent_world = if parent >= 0 {
        model_world_rot(animator, locals, parent as usize)
    } else {
        Quat::IDENTITY
    };
    let local_turn = parent_world.inverse() * turn * parent_world;
    locals[root].rotation = local_turn * locals[root].rotation;
    true
}

/// `from_rotation_arc`, with the angle clamped so a wild pose cannot spin the torso around.
fn clamped_arc(from: Vec3, to: Vec3) -> Quat {
    if from.length_squared() <= 0.0 || to.length_squared() <= 0.0 {
        return Quat::IDENTITY;
    }
    let rotation = Quat::from_rotation_arc(from, to);
    let (axis, angle) = rotation.to_axis_angle();
    if angle <= AIM_TURN_MAX || !axis.is_finite() {
        rotation
    } else {
        Quat::from_axis_angle(axis, AIM_TURN_MAX)
    }
}

/// Pitch the upper body at `lean` radians about the character's right axis, so the rifle follows
/// the camera up and down. The axis is the character's `+X` expressed in the bone's parent frame,
/// which needs the parent chain's rotation from the sampled locals.
fn lean_upper_body(animator: &Animator, locals: &mut [Transform], lean: f32) {
    let lean = lean.clamp(-AIM_LEAN_MAX, AIM_LEAN_MAX);
    let root = animator.upper_root;
    if lean.abs() < 1e-4 || root >= locals.len() {
        return;
    }
    let mut parent_world = Quat::IDENTITY;
    let mut parent = animator.skeleton.parents[root];
    while parent >= 0 {
        parent_world = locals[parent as usize].rotation * parent_world;
        parent = animator.skeleton.parents[parent as usize];
    }
    let axis = parent_world.inverse() * Vec3::X;
    locals[root].rotation = Quat::from_axis_angle(axis, lean) * locals[root].rotation;
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
    buttons: Res<ButtonInput<MouseButton>>,
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
            // The game frame is left-handed (+X forward, +Z up), so a *larger* yaw turns the view
            // left. A rightward mouse move must therefore *decrease* the yaw, or the look (and, now
            // that aiming pins the character to the camera, the rifle) swings the wrong way.
            rig.yaw = (rig.yaw - f64::from(mouse.delta.x) * MOUSE_SENSITIVITY).rem_euclid(360.0);
            rig.pitch = (rig.pitch - f64::from(mouse.delta.y) * MOUSE_SENSITIVITY)
                .clamp(-MAX_PITCH, MAX_PITCH);
        }
    }

    // Holding the right mouse aims down the sights; the camera slides over the shoulder and zooms.
    let aiming = grabbed && buttons.pressed(MouseButton::Right);
    for mut rig in &mut rigs {
        rig.ads = if aiming {
            (rig.ads + time.delta_secs() / ADS_SECONDS).min(1.0)
        } else {
            (rig.ads - time.delta_secs() / ADS_SECONDS).max(0.0)
        };
    }

    let frame_yaw = rigs.single().map(|r| r.yaw).unwrap_or(0.0);

    let mut intent = MoveIntent {
        sprint: keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        jump: keys.just_pressed(KeyCode::Space),
        // Aiming turns the character onto the camera's yaw (so the rifle points where you look)
        // while the input still steers relative to the camera, i.e. the character strafes.
        aim_yaw: aiming.then_some(frame_yaw),
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
        player.aiming = aiming;
        player.character.step(&params, intent, frame_yaw, 0.0, dt);
        transform.translation = position_to_bevy(player.character.position);
        transform.rotation =
            Quat::from_rotation_y(player.character.yaw.to_radians() as f32 + MODEL_YAW_OFFSET);
    }
}

/// Where the camera sits, in metres: behind at `CAMERA_DISTANCE`, and over the right shoulder and
/// closer at `ADS_DISTANCE`/`ADS_SHOULDER` while aiming.
const ADS_DISTANCE: f32 = 1.9;
const ADS_SHOULDER: f32 = 0.55;
const ADS_TARGET_HEIGHT: f32 = 1.62;
/// Field of view (radians) at the hip and while aiming.
const HIP_FOV: f32 = 0.95;
const ADS_FOV: f32 = 0.52;

/// Place the camera on its orbit around the player, aimed at its upper body. The orbit is
/// independent of the player's facing, so the player can be viewed from any side. While aiming the
/// camera slides over the right shoulder, closes in, and zooms, and it looks straight along the aim
/// direction so the reticle and the shots agree.
pub fn follow_camera(
    players: Query<&Player>,
    rigs: Query<&CameraRig>,
    mut cameras: Query<&mut Transform, (With<Camera3d>, Without<Player>)>,
    mut projections: Query<&mut Projection, With<Camera3d>>,
) {
    let (Ok(player), Ok(rig), Ok(mut camera)) =
        (players.single(), rigs.single(), cameras.single_mut())
    else {
        return;
    };
    let ads = rig.ads;
    let dir = look_direction_to_bevy(rig.yaw, rig.pitch);
    let distance = CAMERA_DISTANCE * (1.0 - ads) + ADS_DISTANCE * ads;
    let height = CAMERA_TARGET_HEIGHT * (1.0 - ads) + ADS_TARGET_HEIGHT * ads;
    let target = position_to_bevy(player.character.position) + Vec3::Y * height;
    // Over the shoulder: slide the eye to the right of the aim line, keeping the look direction, so
    // the character sits to the left of the reticle.
    let shoulder = dir.cross(Vec3::Y).normalize_or_zero() * ADS_SHOULDER * ads;
    let eye = target - dir * distance + shoulder;
    *camera = Transform::from_translation(eye).looking_at(eye + dir, Vec3::Y);
    if let Ok(mut projection) = projections.single_mut()
        && let Projection::Perspective(perspective) = &mut *projection
    {
        perspective.fov = HIP_FOV * (1.0 - ads) + ADS_FOV * ads;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    /// The aim layer replaces only the spine-up bones; the pelvis and legs keep the locomotion.
    #[test]
    fn aim_layer_covers_the_upper_body_only() {
        let names: Vec<String> = [
            "Bip001-Pelvis",
            "Bip001-Spine",
            "Bip001-Spine1",
            "Bip001-Spine2",
            "Bip001-L-Thigh",
            "Bip001-L-Clavicle",
            "Bip001-L-UpperArm",
            "Bip001-Head",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let skeleton = Skeleton {
            names,
            parents: vec![-1, 0, 1, 2, 0, 3, 5, 3],
            locals: vec![Transform::default(); 8],
            inverse_bindposes: vec![Mat4::IDENTITY; 8],
            bind: Vec::new(),
        };
        assert_eq!(
            upper_body_mask(&skeleton),
            vec![false, false, true, true, false, true, true, true],
            "the pelvis and thigh keep the locomotion; the spine up takes the aim pose"
        );
    }

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

    /// A cross-fade eases in and out (smoothstep) and clamps at the ends, so a clip change never
    /// overshoots the target pose.
    #[test]
    fn crossfade_smoothstep_eases_and_clamps() {
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
        assert_eq!(smoothstep(2.0), 1.0);
        assert_eq!(smoothstep(-1.0), 0.0);
        assert!((smoothstep(0.5) - 0.5).abs() < 1e-6);
    }

    /// The wiring this module owns: movement follows the input relative to the camera, and the
    /// mouse orbits the camera without turning the character. Proven headless.
    #[test]
    fn movement_follows_the_camera_and_the_mouse_orbits_it() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<ButtonInput<MouseButton>>();
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
        assert!(
            (rig.yaw - 348.0).abs() < 1e-6,
            "a rightward mouse move must turn the view right (yaw decreases, wrapping), got {}",
            rig.yaw
        );
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

        // Camera-relative: forward now follows the orbited camera, so the step gains a +Z (the
        // camera's right) component instead of staying on +X.
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::ZERO;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.update();
        let transform = *app.world().entity(player).get::<Transform>().unwrap();
        assert!(
            transform.translation.z > 0.01,
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
