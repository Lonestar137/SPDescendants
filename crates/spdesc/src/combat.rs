//! Presenting the shooting loop: the trigger, hit-scan tracers, and enemies.
//!
//! The app owns no combat rules: `spdesc-game-core::combat` decides cooldowns, magazines, hit-scan
//! hits, and damage; this module reads the mouse, feeds the simulation a ray from the camera,
//! applies the outcome to enemy entities, and spawns tracers for the shot. Enemies are simple
//! spheres on the map; a real map and enemy AI are later slices.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use spdesc_game_core::combat::{Combat, FireError, Shot, Target, WeaponParams};

use crate::audio::{self, Sounds};
use crate::progress::{self, Career};
use crate::settings::Settings;

/// The volume each cue plays at before the settings trim it, on Bevy's 0..=1 scale.
const SHOT_VOLUME: f32 = 0.5;
const DRY_VOLUME: f32 = 0.6;
const HIT_VOLUME: f32 = 0.55;

/// Seconds a downed target stays down before it pops back up.
const RESPAWN_SECONDS: f32 = 4.0;
/// The colour of a target board, and of a downed one.
const TARGET_COLOUR: Color = Color::srgb(0.86, 0.86, 0.88);
const DOWNED_COLOUR: Color = Color::srgb(0.16, 0.14, 0.15);
/// The board's radius and the post's height, metres.
const BOARD_RADIUS: f32 = 0.45;
const POST_HEIGHT: f32 = 0.62;
/// How far a downed target sinks, so it leaves the field until it returns.
const SINK_DEPTH: f32 = 1.5;

/// A shootable target: its simulation state, how to present it, and its respawn.
#[derive(Component)]
pub struct Enemy {
    pub target: Target,
    /// Where the board stands, and the health it stands back up with.
    pub home: Vec3,
    pub full_health: f64,
    /// Seconds until it pops back up; `0` while it is standing.
    pub respawn: f32,
    /// The board's material, so this target alone can be dimmed. It is on the board *child* (the
    /// anchor entity carries the transform the fall is written to), so the handle is kept here
    /// rather than reached through a query.
    pub board: Handle<StandardMaterial>,
}

/// A fading tracer line between two Bevy-space points.
#[derive(Component)]
pub struct Tracer {
    pub ttl: f32,
    /// Its starting time to live, so the emissive can fade proportionally.
    pub life: f32,
}

/// The player's weapon: the tuning and the simulation state.
#[derive(Component, Default)]
pub struct Weapon {
    pub params: WeaponParams,
    pub combat: Combat,
}

/// A short-lived impact spark at the hit point.
#[derive(Component)]
pub struct Impact {
    pub ttl: f32,
}

/// A short-lived muzzle flash at the barrel's tip.
#[derive(Component)]
pub struct Flash {
    pub ttl: f32,
}

/// How far (metres) a shot that hits nothing still draws its tracer, so a miss is a short line
/// rather than a beam across the map.
const MISS_TRACER_LENGTH: f32 = 45.0;
/// Seconds a tracer, a muzzle flash and an impact live.
const TRACER_TTL: f32 = 0.10;
const FLASH_TTL: f32 = 0.05;
const IMPACT_TTL: f32 = 0.12;
/// Recoil travel per shot (metres, back along the barrel) and how fast it eases back, per second.
const RECOIL_KICK: f32 = 0.05;
const RECOIL_DECAY: f32 = 10.0;

/// The weapon model's socket node on the right hand; it carries the recoil kick.
#[derive(Component, Default)]
pub struct WeaponRig {
    /// Recoil displacement from rest, metres, in the socket's local frame (back along `-X`).
    pub kick: Vec3,
}

/// The barrel's tip, where a shot's tracer and muzzle flash start.
#[derive(Component)]
pub struct Muzzle;

/// Advance the weapon timers by the frame delta, ease the weapon's recoil back to rest, and ring the
/// magazine seating when a reload finishes.
pub fn tick_weapon(
    mut commands: Commands,
    time: Res<Time>,
    sounds: Res<Sounds>,
    mut weapons: Query<&mut Weapon>,
    mut rigs: Query<(&mut WeaponRig, &mut Transform)>,
) {
    let dt = (time.delta_secs() as f64).clamp(0.0, 0.05);
    for mut weapon in &mut weapons {
        let params = weapon.params;
        let was_reloading = weapon.combat.reload_remaining;
        weapon.combat.tick(&params, dt);
        if was_reloading > 0.0 && weapon.combat.reload_remaining == 0.0 {
            audio::play(&mut commands, &sounds.reload_end, 0.7);
        }
    }
    for (mut rig, mut transform) in &mut rigs {
        rig.kick *= (1.0 - RECOIL_DECAY * dt as f32).clamp(0.0, 1.0);
        if rig.kick.length_squared() < 1e-7 {
            rig.kick = Vec3::ZERO;
        }
        transform.translation = rig.kick;
    }
}

/// Start a reload and click, when one actually begins.
fn begin_reload(commands: &mut Commands, sounds: &Sounds, weapon: &mut Weapon) {
    let params = weapon.params;
    let was = weapon.combat.reload_remaining;
    weapon.combat.start_reload(&params);
    if was == 0.0 && weapon.combat.reload_remaining > 0.0 {
        audio::play(commands, &sounds.reload_start, 0.7);
    }
}

/// Fire on the left mouse button (held, for automatic fire); `R` reloads. The ray is the camera's
/// forward direction from the camera, so shots land where the view points.
pub fn fire_weapon(
    mut commands: Commands,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&CursorOptions, With<PrimaryWindow>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    muzzles: Query<(Entity, &GlobalTransform), With<Muzzle>>,
    mut rigs: Query<&mut WeaponRig>,
    sounds: Res<Sounds>,
    settings: Res<Settings>,
    mut career: ResMut<Career>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut weapons: Query<&mut Weapon>,
    mut enemies: Query<(Entity, &GlobalTransform, &mut Enemy)>,
) {
    let grabbed = windows
        .single()
        .map(|o| o.grab_mode != CursorGrabMode::None)
        .unwrap_or(true);
    let Ok(camera) = cameras.single() else {
        return;
    };
    let origin_bevy = camera.translation();
    let forward_bevy = camera.forward();
    // Shots present from the barrel's tip, not from the eye, so the tracer reads as the gun firing.
    let muzzle = muzzles
        .single()
        .ok()
        .map(|(entity, tf)| (entity, tf.translation()));

    for mut weapon in &mut weapons {
        if keys.just_pressed(KeyCode::KeyR) {
            begin_reload(&mut commands, &sounds, &mut weapon);
        }
        if !grabbed || !buttons.pressed(MouseButton::Left) {
            continue;
        }
        // Build the enemy list in game space, remembering the entity for each index.
        let mut list: Vec<Target> = Vec::new();
        let mut entities: Vec<Entity> = Vec::new();
        for (entity, _tf, enemy) in &enemies {
            list.push(enemy.target);
            entities.push(entity);
        }
        let params = weapon.params;
        let origin = to_game(origin_bevy);
        let direction = dir_to_game(*forward_bevy);
        match weapon
            .combat
            .try_fire(&params, origin, direction, &mut list)
        {
            Ok(Shot::Hit {
                index,
                point,
                damage,
                killed,
            }) => {
                if let Ok((_e, _tf, mut enemy)) = enemies.get_mut(entities[index]) {
                    enemy.target.health -= damage;
                    enemy.target.health = enemy.target.health.max(0.0);
                }
                // A hit is worth experience, and a target that went down is worth a kill's.
                progress::grant(&mut career, progress::HIT_XP);
                if killed {
                    progress::grant(&mut career, progress::KILL_XP);
                }
                let start = muzzle.map_or(origin_bevy, |(_, at)| at);
                let hit = to_bevy(point);
                audio::play(&mut commands, &sounds.shot, settings.shot(SHOT_VOLUME));
                // The hitmarker, whether or not the hit finished the target.
                audio::play(&mut commands, &sounds.hit, settings.cue(HIT_VOLUME));
                spawn_tracer(&mut commands, &mut meshes, &mut materials, start, hit);
                spawn_impact(&mut commands, &mut meshes, &mut materials, hit);
                if let Some((entity, _)) = muzzle {
                    spawn_flash(&mut commands, &mut meshes, &mut materials, entity);
                }
                kick(&mut rigs);
            }
            Ok(Shot::Miss { .. }) => {
                // The ray left the muzzle's neighbourhood; draw the miss tracer from the muzzle.
                let start = muzzle.map_or(origin_bevy, |(_, at)| at);
                let end = start + forward_bevy * MISS_TRACER_LENGTH;
                audio::play(&mut commands, &sounds.shot, settings.shot(SHOT_VOLUME));
                spawn_tracer(&mut commands, &mut meshes, &mut materials, start, end);
                if let Some((entity, _)) = muzzle {
                    spawn_flash(&mut commands, &mut meshes, &mut materials, entity);
                }
                kick(&mut rigs);
            }
            Err(FireError::Empty) => {
                audio::play(&mut commands, &sounds.dry, settings.shot(DRY_VOLUME));
                begin_reload(&mut commands, &sounds, &mut weapon);
            }
            Err(_) => {}
        }
    }
}

/// Fade and despawn tracers, flashes and impacts. Each owns its material, so it fades on its own.
pub fn age_effects(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tracers: Query<(Entity, &mut Tracer, &MeshMaterial3d<StandardMaterial>)>,
    mut flashes: Query<(Entity, &mut Flash, &MeshMaterial3d<StandardMaterial>)>,
    mut impacts: Query<(Entity, &mut Impact)>,
) {
    let dt = time.delta_secs();
    for (entity, mut tracer, material) in &mut tracers {
        tracer.ttl -= dt;
        let fade = (tracer.ttl / tracer.life).clamp(0.0, 1.0);
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.emissive = LinearRgba::new(4.0 * fade, 3.4 * fade, 1.2 * fade, 1.0);
        }
        if tracer.ttl <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
    for (entity, mut flash, material) in &mut flashes {
        flash.ttl -= dt;
        let fade = (flash.ttl / FLASH_TTL).clamp(0.0, 1.0);
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.emissive = LinearRgba::new(9.0 * fade, 6.0 * fade, 2.0 * fade, 1.0);
        }
        if flash.ttl <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
    for (entity, mut impact) in &mut impacts {
        impact.ttl -= dt;
        if impact.ttl <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

/// Kick the weapon back along its barrel (`-X`) for one shot.
fn kick(rigs: &mut Query<&mut WeaponRig>) {
    for mut rig in rigs {
        rig.kick.x -= RECOIL_KICK;
    }
}

/// What a target's down/respawn timer did this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Respawn {
    /// Nothing changed: alive, or still waiting to come back.
    Waiting,
    /// It has just been killed.
    Downed,
    /// Its time is up and it stands again.
    Revived,
}

/// Advance one target's down/respawn timer by `dt`. A target with health stays up; a killed one
/// goes down once, stays down for [`RESPAWN_SECONDS`], then revives once. `respawn` is `0` while it
/// stands.
fn advance_respawn(health: f64, respawn: &mut f32, dt: f32) -> Respawn {
    if health > 0.0 {
        return Respawn::Waiting;
    }
    if *respawn <= 0.0 {
        *respawn = RESPAWN_SECONDS;
        return Respawn::Downed;
    }
    *respawn -= dt;
    if *respawn > 0.0 {
        return Respawn::Waiting;
    }
    *respawn = 0.0;
    Respawn::Revived
}

/// Take down a target that has been killed — sound it, tip it over, dim it — and stand it back up,
/// re-armed, when its timer elapses. A downed target is skipped by the hit scan (`is_alive`), so
/// shots pass through it until it returns.
pub fn update_targets(
    mut commands: Commands,
    time: Res<Time>,
    sounds: Res<Sounds>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut enemies: Query<(&mut Enemy, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (mut enemy, mut tf) in &mut enemies {
        match advance_respawn(enemy.target.health, &mut enemy.respawn, dt) {
            Respawn::Waiting => {}
            Respawn::Downed => {
                audio::play(&mut commands, &sounds.kill, 0.7);
                tf.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
                tf.translation = enemy.home - Vec3::Y * SINK_DEPTH;
                if let Some(mut board) = materials.get_mut(&enemy.board) {
                    board.base_color = DOWNED_COLOUR;
                }
            }
            Respawn::Revived => {
                enemy.target.health = enemy.full_health;
                tf.translation = enemy.home;
                tf.rotation = Quat::IDENTITY;
                if let Some(mut board) = materials.get_mut(&enemy.board) {
                    board.base_color = TARGET_COLOUR;
                }
            }
        }
    }
}

fn spawn_tracer(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    from: Vec3,
    to: Vec3,
) {
    let delta = to - from;
    let length = delta.length().max(0.02);
    let mid = from + delta * 0.5;
    let rotation = Quat::from_rotation_arc(Vec3::Y, delta.normalize_or(Vec3::Y));
    let mesh = meshes.add(Cuboid::new(0.03, length, 0.03));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.9, 0.4),
        emissive: LinearRgba::new(4.0, 3.4, 1.2, 1.0),
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_translation(mid).with_rotation(rotation),
        Tracer {
            ttl: TRACER_TTL,
            life: TRACER_TTL,
        },
    ));
}

/// A bright flash on the muzzle, stretched along the barrel. It hangs off the muzzle entity so it
/// inherits the weapon's frame rather than lying flat across it.
fn spawn_flash(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    muzzle: Entity,
) {
    let mesh = meshes.add(Sphere::new(0.06));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.9, 0.6),
        emissive: LinearRgba::new(9.0, 6.0, 2.0, 1.0),
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_scale(Vec3::new(1.9, 0.8, 0.8)),
        Flash { ttl: FLASH_TTL },
        ChildOf(muzzle),
    ));
}

fn spawn_impact(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
) {
    let mesh = meshes.add(Sphere::new(0.12));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.7, 0.2),
        emissive: LinearRgba::new(8.0, 5.0, 1.5, 1.0),
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_translation(at),
        Impact { ttl: IMPACT_TTL },
    ));
}

/// Stand up a row of targets on the map. Game-space positions (cm).
///
/// Each is a bullseye board on a post. The post carries the mesh and the board a separate one, so
/// the hit scan's sphere (the `Target` radius, centred on the board) matches what is drawn.
pub fn spawn_enemies(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    let board_texture = images.add(bullseye_image(256));
    let post = meshes.add(Cuboid::new(0.07, POST_HEIGHT, 0.07));
    let board = meshes.add(Cylinder::new(BOARD_RADIUS, 0.06));
    // Spread across the field rather than down the muzzle line, so they read from the start.
    for (x, y) in [
        (1200.0f64, 400.0f64),
        (1500.0, -700.0),
        (1500.0, 700.0),
        (2600.0, 1100.0),
        (2600.0, -1100.0),
        (2400.0, 0.0),
        (3400.0, 600.0),
        (3400.0, -600.0),
    ] {
        let center = [x, y, 100.0];
        let home = to_bevy(center);
        // The board stands upright and faces the player's start, so a bullseye reads head-on.
        let facing = Vec3::new(-home.x, 0.0, -home.z).normalize_or(Vec3::X);
        let board_rotation = Quat::from_rotation_arc(Vec3::Y, facing);
        // One material per target: a downed board dims only itself.
        let material = materials.add(StandardMaterial {
            base_color: TARGET_COLOUR,
            base_color_texture: Some(board_texture.clone()),
            perceptual_roughness: 0.75,
            ..default()
        });
        commands
            .spawn((
                Transform::from_translation(home),
                Visibility::default(),
                Enemy {
                    target: Target::new(center, 70.0, 600.0),
                    home,
                    full_health: 600.0,
                    respawn: 0.0,
                    board: material.clone(),
                },
            ))
            .with_children(|parent| {
                parent.spawn((
                    Mesh3d(board.clone()),
                    MeshMaterial3d(material),
                    Transform::from_rotation(board_rotation),
                ));
                parent.spawn((
                    Mesh3d(post.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgb(0.22, 0.23, 0.26),
                        perceptual_roughness: 0.8,
                        ..default()
                    })),
                    Transform::from_translation(Vec3::NEG_Y * (POST_HEIGHT + BOARD_RADIUS) * 0.5),
                ));
            });
    }
}

/// A bullseye: concentric rings, dark centre out to a light rim, so a target reads at a distance.
fn bullseye_image(size: u32) -> Image {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    let centre = (size - 1) as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - centre;
            let dy = y as f32 - centre;
            let radius = dx.hypot(dy) / centre;
            let colour = if radius > 1.0 {
                [0.86, 0.86, 0.88, 1.0]
            } else if radius < 0.18 {
                [0.10, 0.10, 0.12, 1.0]
            } else if radius < 0.38 {
                [0.78, 0.16, 0.16, 1.0]
            } else if radius < 0.55 {
                [0.95, 0.95, 0.95, 1.0]
            } else if radius < 0.75 {
                [0.78, 0.16, 0.16, 1.0]
            } else {
                [0.95, 0.95, 0.95, 1.0]
            };
            pixels.extend_from_slice(&colour.map(|c| (c * 255.0) as u8));
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Game-space (cm) to Bevy-space (m): `(x, y, z)` → `(x, z, -y) * 0.01`.
fn to_bevy(p: [f64; 3]) -> Vec3 {
    Vec3::new(
        (p[0] * 0.01) as f32,
        (p[2] * 0.01) as f32,
        (-p[1] * 0.01) as f32,
    )
}

fn to_game(v: Vec3) -> [f64; 3] {
    [
        f64::from(v.x) / 0.01,
        -f64::from(v.z) / 0.01,
        f64::from(v.y) / 0.01,
    ]
}

fn dir_to_game(v: Vec3) -> [f64; 3] {
    [f64::from(v.x), -f64::from(v.z), f64::from(v.y)]
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;

    /// A killed target goes down exactly once, stays down for the full delay, then stands back up
    /// once — so the kill sound and the tip-over cannot repeat every frame.
    #[test]
    fn a_killed_target_goes_down_and_revives_once() {
        let dt = 1.0 / 60.0;
        let mut health = 600.0;
        let mut respawn = 0.0;
        let mut downed = 0;
        let mut revived = 0;
        for _ in 0..(60.0 * 10.0) as i32 {
            match advance_respawn(health, &mut respawn, dt) {
                Respawn::Waiting => {}
                Respawn::Downed => downed += 1,
                Respawn::Revived => {
                    revived += 1;
                    health = 600.0;
                }
            }
            // An alive target's timer is untouched; a downed one counts down.
            assert!((0.0..=RESPAWN_SECONDS).contains(&respawn));
        }
        assert_eq!((downed, revived), (0, 0), "an untouched target stays up");

        health = 0.0;
        for _ in 0..(60.0 * 10.0) as i32 {
            match advance_respawn(health, &mut respawn, dt) {
                Respawn::Waiting => {}
                Respawn::Downed => downed += 1,
                Respawn::Revived => {
                    revived += 1;
                    health = 600.0;
                }
            }
        }
        assert_eq!((downed, revived), (1, 1), "one fall, one revival");

        // It is down for the whole delay, and not a frame less.
        health = 0.0;
        respawn = 0.0;
        let mut event = advance_respawn(health, &mut respawn, dt);
        assert_eq!(event, Respawn::Downed);
        let mut elapsed = 0.0;
        while event != Respawn::Revived && elapsed < 2.0 * RESPAWN_SECONDS {
            event = advance_respawn(health, &mut respawn, dt);
            elapsed += dt;
        }
        assert_eq!(event, Respawn::Revived);
        assert!(
            (elapsed - RESPAWN_SECONDS).abs() <= 2.0 * dt,
            "down for {elapsed}s, expected about {RESPAWN_SECONDS}s"
        );
        assert_eq!(respawn, 0.0, "the timer resets once it is back up");
    }

    /// A killed target leaves the field and comes back. This drives the real system over an entity
    /// shaped like a spawned one — the anchor carries the transform the fall is written to, and the
    /// board (with its material) hangs below it — so a query that reached for the board's own
    /// components would match nothing and silently do nothing.
    #[test]
    fn a_killed_target_leaves_the_field_and_returns() {
        let mut app = App::new();
        app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_resource::<Time>();
        app.init_asset::<StandardMaterial>();
        app.init_asset::<AudioSource>();
        // The kill cue's handles are irrelevant here; only the board's material is reached.
        app.insert_resource(Sounds {
            shot: default(),
            dry: default(),
            hit: default(),
            kill: default(),
            reload_start: default(),
            reload_end: default(),
            footstep: default(),
            jump: default(),
            land: default(),
            promotion: default(),
            prestige: default(),
        });
        app.add_systems(Update, update_targets);

        let home = Vec3::new(15.0, 1.0, 0.0);
        let board = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let target = app
            .world_mut()
            .spawn((
                Transform::from_translation(home),
                Enemy {
                    target: Target::new([1500.0, 0.0, 100.0], 70.0, 0.0),
                    home,
                    full_health: 600.0,
                    respawn: 0.0,
                    board,
                },
            ))
            .id();

        app.update();
        let sunk = app
            .world()
            .entity(target)
            .get::<Transform>()
            .unwrap()
            .translation;
        assert!(
            sunk.y < home.y - 1.0,
            "a downed target sinks out of the field, got {sunk:?}"
        );

        for _ in 0..(RESPAWN_SECONDS / 0.1) as i32 + 2 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_millis(100));
            app.update();
        }
        let enemy = app.world().entity(target).get::<Enemy>().unwrap();
        assert_eq!(enemy.target.health, 600.0, "it stands back up armed");
        let stood = app
            .world()
            .entity(target)
            .get::<Transform>()
            .unwrap()
            .translation;
        assert_eq!(stood, home, "and back where it stood");
    }

    #[test]
    fn game_and_bevy_spaces_round_trip() {
        let p = [123.0, -45.0, 678.0];
        let round = to_game(to_bevy(p));
        for (a, b) in p.iter().zip(round.iter()) {
            assert!((a - b).abs() < 1e-3);
        }
    }
}
