//! Presenting the shooting loop: the trigger, hit-scan tracers, and enemies.
//!
//! The app owns no combat rules: `spdesc-game-core::combat` decides cooldowns, magazines, hit-scan
//! hits, and damage; this module reads the mouse, feeds the simulation a ray from the camera,
//! applies the outcome to enemy entities, and spawns tracers for the shot. Enemies are simple
//! spheres on the map; a real map and enemy AI are later slices.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use spdesc_game_core::combat::{Combat, FireError, Shot, Target, WeaponParams};

/// The enemy's simulation state plus how to present it.
#[derive(Component)]
pub struct Enemy {
    pub target: Target,
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

/// Advance the weapon timers by the frame delta, and ease the weapon's recoil back to rest.
pub fn tick_weapon(
    time: Res<Time>,
    mut weapons: Query<&mut Weapon>,
    mut rigs: Query<(&mut WeaponRig, &mut Transform)>,
) {
    let dt = (time.delta_secs() as f64).clamp(0.0, 0.05);
    for mut weapon in &mut weapons {
        let params = weapon.params;
        weapon.combat.tick(&params, dt);
    }
    for (mut rig, mut transform) in &mut rigs {
        rig.kick *= (1.0 - RECOIL_DECAY * dt as f32).clamp(0.0, 1.0);
        if rig.kick.length_squared() < 1e-7 {
            rig.kick = Vec3::ZERO;
        }
        transform.translation = rig.kick;
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
    muzzles: Query<&GlobalTransform, With<Muzzle>>,
    mut rigs: Query<&mut WeaponRig>,
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
    let muzzle = muzzles.single().map(GlobalTransform::translation).ok();

    for mut weapon in &mut weapons {
        if keys.just_pressed(KeyCode::KeyR) {
            let params = weapon.params;
            weapon.combat.start_reload(&params);
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
                let _ = killed;
                let start = muzzle.unwrap_or(origin_bevy);
                let hit = to_bevy(point);
                spawn_tracer(&mut commands, &mut meshes, &mut materials, start, hit);
                spawn_impact(&mut commands, &mut meshes, &mut materials, hit);
                spawn_flash(&mut commands, &mut meshes, &mut materials, start);
                kick(&mut rigs);
            }
            Ok(Shot::Miss { .. }) => {
                // The ray left the muzzle's neighbourhood; draw the miss tracer from the muzzle.
                let start = muzzle.unwrap_or(origin_bevy);
                let end = start + forward_bevy * MISS_TRACER_LENGTH;
                spawn_tracer(&mut commands, &mut meshes, &mut materials, start, end);
                spawn_flash(&mut commands, &mut meshes, &mut materials, start);
                kick(&mut rigs);
            }
            Err(FireError::Empty) => {
                let params = weapon.params;
                weapon.combat.start_reload(&params);
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

/// Darken a dead enemy and sink it, so a kill reads.
pub fn retire_dead_enemies(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut enemies: Query<(
        Entity,
        &mut Enemy,
        &MeshMaterial3d<StandardMaterial>,
        &mut Transform,
    )>,
) {
    for (entity, enemy, material, mut tf) in &mut enemies {
        if enemy.target.health <= 0.0 {
            if let Some(mut mat) = materials.get_mut(&material.0) {
                mat.base_color = Color::srgb(0.1, 0.1, 0.12);
            }
            // Lie the corpse over.
            tf.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
            tf.translation.y = 0.4;
            commands.entity(entity).remove::<Enemy>();
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

/// A bright unlit flash at the muzzle, angled arbitrarily so it does not read as a flat card.
fn spawn_flash(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    at: Vec3,
) {
    let mesh = meshes.add(Cuboid::new(0.16, 0.16, 0.16));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.85, 0.5),
        emissive: LinearRgba::new(9.0, 6.0, 2.0, 1.0),
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_translation(at)
            .with_rotation(Quat::from_rotation_x(0.6))
            .with_scale(Vec3::new(1.0, 1.0, 0.4)),
        Flash { ttl: FLASH_TTL },
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

/// Spawn a row of enemy targets on the map. Game-space positions (cm).
pub fn spawn_enemies(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let mesh = meshes.add(Capsule3d::new(0.4, 1.1));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.7, 0.25, 0.3),
        perceptual_roughness: 0.7,
        ..default()
    });
    for (i, (x, y)) in [
        (1500.0f64, 0.0f64),
        (1500.0, 600.0),
        (1500.0, -600.0),
        (2600.0, 300.0),
    ]
    .into_iter()
    .enumerate()
    {
        let center = [x, y, 100.0];
        let _ = i;
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(to_bevy(center)),
            Enemy {
                target: Target::new(center, 70.0, 600.0),
            },
        ));
    }
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
    use super::*;

    #[test]
    fn game_and_bevy_spaces_round_trip() {
        let p = [123.0, -45.0, 678.0];
        let round = to_game(to_bevy(p));
        for (a, b) in p.iter().zip(round.iter()) {
            assert!((a - b).abs() < 1e-3);
        }
    }
}
