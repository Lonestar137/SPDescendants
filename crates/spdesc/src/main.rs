//! `spdesc` — the SPDescendants client.
//!
//! Presentation only: the simulation lives in `spdesc-game-core` and is Bevy-free, and the asset
//! formats live in `spdesc-data`. The app resolves the read-only `GAME_DATA` folder through
//! `spdesc_game_core::game_data`, builds the Ultimate Bunny's meshes and textures from the
//! player's install at startup, and presents a movable character on a flat ground plane.

mod combat;
mod content;
mod player;

use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};

use combat::{Weapon, age_effects, fire_weapon, retire_dead_enemies, spawn_enemies, tick_weapon};
use content::{CharacterAssets, DataSource, MaterialSpec};
use player::{
    Animator, CameraRig, Player, animate_locomotion, drive_player, follow_camera, grab_cursor,
    toggle_cursor,
};
use spdesc_data::texture::Texture;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "SPDescendants".into(),
                ..default()
            }),
            ..default()
        }))
        // A gentle sky fill and a soft clear colour: the default near-white ambient at full
        // directional illuminance washed the character's colours out.
        .insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.62, 0.70, 0.82),
            brightness: 130.0,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.46, 0.55, 0.68)))
        .add_systems(Startup, (setup_scene, grab_cursor, spawn_character))
        .add_systems(
            Update,
            (
                toggle_cursor,
                drive_player,
                animate_locomotion,
                follow_camera,
                tick_weapon,
                fire_weapon,
                age_effects,
                retire_dead_enemies,
            )
                .chain(),
        )
        .run();
}

/// Camera, light, and the flat ground the MVP plays on. The ground is a tiled checker and there
/// are a few pillars, so the third-person camera's horizontal orbit has something to read against.
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        CameraRig::default(),
        Transform::from_xyz(0.0, 2.5, -5.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));

    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.0, 0.97, 0.90),
            illuminance: 4_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0)),
    ));

    // A large checker ground plane at Y = 0 (200 m square), the "basic map of any kind".
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(200.0, 200.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(images.add(checker_image(512, 64))),
            uv_transform: Affine2::from_scale(Vec2::splat(50.0)),
            perceptual_roughness: 1.0,
            ..default()
        })),
    ));

    // Reference pillars: without something to read against, the camera's horizontal orbit is
    // invisible on a uniform plane.
    let pillar = meshes.add(Cuboid::new(1.0, 4.0, 1.0));
    let pillar_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.55, 0.30, 0.35),
        perceptual_roughness: 0.6,
        ..default()
    });
    for (x, z) in [(-8.0, -8.0), (8.0, -8.0), (-8.0, 8.0), (8.0, 8.0)] {
        commands.spawn((
            Mesh3d(pillar.clone()),
            MeshMaterial3d(pillar_material.clone()),
            Transform::from_xyz(x, 2.0, z),
        ));
    }

    spawn_enemies(&mut commands, &mut meshes, &mut materials);
}

/// A two-tone checker image with `cell`-pixel squares.
fn checker_image(size: u32, cell: u32) -> Image {
    let mut data = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let light = ((x / cell) + (y / cell)).is_multiple_of(2);
            let v = if light { 0x6a } else { 0x50 };
            let o = ((y * size + x) * 4) as usize;
            data[o] = v;
            data[o + 1] = v + 6;
            data[o + 2] = v;
            data[o + 3] = 255;
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Load the Ultimate Bunny from `GAME_DATA` and spawn the player, or fall back to a capsule so the
/// program is still playable when the data, key, or mapping is absent.
fn spawn_character(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inverse_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    let assets = match build_character() {
        Ok(assets) => Some(assets),
        Err(error) => {
            warn!("could not build the character from GAME_DATA ({error:#}); using a capsule");
            None
        }
    };

    let player = commands
        .spawn((
            Player::default(),
            Weapon::default(),
            Transform::default(),
            Visibility::default(),
        ))
        .id();

    match assets {
        Some(CharacterAssets {
            parts,
            materials: specs,
            skeleton,
            head_attach,
            feet_offset,
            animation,
        }) => {
            info!(
                "Ultimate Bunny: {} section mesh(es), {} material(s), {} bones, feet offset {feet_offset:.3} m",
                parts.len(),
                specs.len(),
                skeleton.names.len()
            );
            let handles: Vec<_> = specs
                .into_iter()
                .map(|spec| materials.add(build_material(&mut images, spec)))
                .collect();
            let (head_parts, body_parts): (Vec<_>, Vec<_>) =
                parts.into_iter().partition(|part| part.under_head);

            // One entity per bone, parented into the hierarchy under the player, so the skinned
            // mesh's joint matrices carry the player's world transform. The `Animator` poses them
            // from the decoded clips.
            let mut bones: Vec<Entity> = Vec::with_capacity(skeleton.names.len());
            for local in &skeleton.locals {
                bones.push(commands.spawn((*local, Visibility::default())).id());
            }
            for (i, parent) in skeleton.parents.iter().enumerate() {
                let parent_entity = if *parent >= 0 {
                    bones[*parent as usize]
                } else {
                    player
                };
                commands.entity(bones[i]).insert(ChildOf(parent_entity));
            }
            let inverse = inverse_bindposes.add(SkinnedMeshInverseBindposes::from(
                skeleton.inverse_bindposes.clone(),
            ));

            // A stand-in weapon on the right hand bone, so the character visibly carries a gun. A
            // real weapon mesh from GAME_DATA is a later slice (its own mesh and materials).
            if let Some(hand) = skeleton.names.iter().position(|n| n == "Bip001-R-Hand") {
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.06, 0.10, 0.42))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgb(0.12, 0.13, 0.15),
                        metallic: 0.85,
                        perceptual_roughness: 0.35,
                        ..default()
                    })),
                    Transform::from_xyz(0.0, 0.0, 0.12),
                    ChildOf(bones[hand]),
                ));
            }

            if let Some(animation) = animation {
                commands.entity(player).insert(Animator {
                    clips: animation.clips,
                    idle: animation.idle,
                    walk: animation.walk,
                    run: animation.run,
                    jump: animation.jump,
                    fall: animation.fall,
                    land: animation.land,
                    bone_to_track: animation.bone_to_track,
                    bones: bones.clone(),
                    skeleton,
                    time: 0.0,
                    air_timer: 0.0,
                    fall_timer: 0.0,
                    land_timer: 0.0,
                    was_grounded: true,
                });
            }

            // The body: skinned sub-meshes on the player. The head: static sub-meshes on the head
            // attach node, so they follow the head bone.
            commands.entity(player).with_children(|parent| {
                for part in body_parts {
                    let mut child = parent.spawn((
                        Mesh3d(meshes.add(part.mesh)),
                        MeshMaterial3d(handles[part.material].clone()),
                        Transform::from_xyz(0.0, feet_offset, 0.0),
                    ));
                    if part.skinned {
                        child.insert(SkinnedMesh {
                            inverse_bindposes: inverse.clone(),
                            joints: bones.clone(),
                        });
                    }
                }
            });
            if let Some((index, local)) = head_attach {
                let node = commands
                    .spawn((local, Visibility::default(), ChildOf(bones[index])))
                    .id();
                commands.entity(node).with_children(|parent| {
                    for part in head_parts {
                        parent.spawn((
                            Mesh3d(meshes.add(part.mesh)),
                            MeshMaterial3d(handles[part.material].clone()),
                            Transform::default(),
                        ));
                    }
                });
            }
        }
        None => {
            commands.entity(player).with_children(|parent| {
                parent.spawn((
                    Mesh3d(meshes.add(Capsule3d::new(0.4, 1.0))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgb(0.8, 0.3, 0.6),
                        ..default()
                    })),
                    Transform::from_xyz(0.0, 0.9, 0.0),
                ));
            });
        }
    }
}

/// Turn a decoded [`Texture`] into a Bevy image. Base colours are sRGB; normal and packed maps
/// carry linear data.
fn to_image(images: &mut Assets<Image>, texture: Texture, srgb: bool) -> Handle<Image> {
    images.add(Image::new(
        Extent3d {
            width: texture.width,
            height: texture.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texture.rgba,
        if srgb {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Rgba8Unorm
        },
        RenderAssetUsages::default(),
    ))
}

/// Build a Bevy material from a slot's decoded maps: base colour (sRGB), the game's packed map as
/// both the metallic/roughness and occlusion texture, and the BC5 normal map.
fn build_material(images: &mut Assets<Image>, spec: MaterialSpec) -> StandardMaterial {
    let base_color_texture = spec.base.map(|t| to_image(images, t, true));
    let metallic_roughness_texture = spec.packed.map(|t| to_image(images, t, false));
    let occlusion_texture = metallic_roughness_texture.clone();
    let normal_map_texture = spec.normal.map(|t| to_image(images, t, false));
    StandardMaterial {
        base_color: Color::srgba(spec.tint[0], spec.tint[1], spec.tint[2], spec.tint[3]),
        base_color_texture,
        metallic_roughness_texture,
        occlusion_texture,
        normal_map_texture,
        perceptual_roughness: spec.roughness,
        metallic: spec.metallic,
        // Glass stands in as a translucent tint.
        alpha_mode: if spec.tint[3] < 1.0 {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        },
        ..default()
    }
}

/// Open the container and the local state, then decode the character.
fn build_character() -> anyhow::Result<CharacterAssets> {
    let paks =
        spdesc_data::paks_dir().ok_or_else(|| anyhow::anyhow!("no GAME_DATA folder found"))?;
    let key = spdesc_data::key::aes_key()
        .ok_or_else(|| anyhow::anyhow!("no container key (spdesc-config/keys.json)"))?;
    let schema =
        spdesc_data::mapping::load().map_err(|e| anyhow::anyhow!("no property mapping: {e}"))?;
    let mut data = DataSource::open(&paks, key, schema)?;
    content::load_bunny(&mut data)
}
