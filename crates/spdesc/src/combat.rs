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

/// Advance the weapon timers by the frame delta.
pub fn tick_weapon(time: Res<Time>, mut weapons: Query<&mut Weapon>) {
    let dt = (time.delta_secs() as f64).clamp(0.0, 0.05);
    for mut weapon in &mut weapons {
        let params = weapon.params;
        weapon.combat.tick(&params, dt);
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
                let hit = to_bevy(point);
                let _ = killed;
                spawn_tracer(&mut commands, &mut meshes, &mut materials, origin_bevy, hit);
                spawn_impact(&mut commands, &mut meshes, &mut materials, hit);
            }
            Ok(Shot::Miss { point }) => {
                spawn_tracer(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    origin_bevy,
                    to_bevy(point),
                );
            }
            Err(FireError::Empty) => {
                let params = weapon.params;
                weapon.combat.start_reload(&params);
            }
            Err(_) => {}
        }
    }
}

/// Fade and despawn tracers and impacts.
pub fn age_effects(
    mut commands: Commands,
    time: Res<Time>,
    mut tracers: Query<(Entity, &mut Tracer, &mut Transform)>,
    mut impacts: Query<(Entity, &mut Impact)>,
) {
    let dt = time.delta_secs();
    for (entity, mut tracer, mut tf) in &mut tracers {
        tracer.ttl -= dt;
        // Shrink the tracer towards its origin as it fades.
        tf.scale.y *= 0.6;
        tf.scale.z *= 0.6;
        if tracer.ttl <= 0.0 {
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
        Tracer { ttl: 0.06 },
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
        Impact { ttl: 0.12 },
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
