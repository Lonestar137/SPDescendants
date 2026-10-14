//! `spdesc` — the SPDescendants client.
//!
//! Presentation only: the simulation lives in `spdesc-game-core` and is Bevy-free, and the asset
//! formats live in `spdesc-data`. The app resolves the read-only `GAME_DATA` folder through
//! `spdesc_game_core::game_data`, builds the Ultimate Bunny's meshes and textures from the
//! player's install at startup, and presents a movable character on a flat ground plane.

mod content;
mod player;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use content::{CharacterAssets, DataSource, MaterialSpec};
use player::{Player, drive_player, follow_camera, grab_cursor, toggle_cursor};
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
        .add_systems(Update, (toggle_cursor, drive_player, follow_camera).chain())
        .run();
}

/// Camera, light, and the flat ground the MVP plays on.
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
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

    // A large ground plane at Y = 0 (200 m square), the "basic map of any kind".
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(200.0, 200.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.34, 0.30),
            perceptual_roughness: 1.0,
            ..default()
        })),
    ));
}

/// Load the Ultimate Bunny from `GAME_DATA` and spawn the player, or fall back to a capsule so the
/// program is still playable when the data, key, or mapping is absent.
fn spawn_character(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let assets = match build_character() {
        Ok(assets) => Some(assets),
        Err(error) => {
            warn!("could not build the character from GAME_DATA ({error:#}); using a capsule");
            None
        }
    };

    let mut entity = commands.spawn((
        Player::default(),
        Transform::default(),
        Visibility::default(),
    ));

    match assets {
        Some(CharacterAssets {
            parts,
            materials: specs,
            feet_offset,
        }) => {
            info!(
                "Ultimate Bunny: {} section mesh(es), {} material(s), feet offset {feet_offset:.3} m",
                parts.len(),
                specs.len()
            );
            let handles: Vec<_> = specs
                .into_iter()
                .map(|spec| materials.add(build_material(&mut images, spec)))
                .collect();
            entity.with_children(|parent| {
                for part in parts {
                    parent.spawn((
                        Mesh3d(meshes.add(part.mesh)),
                        MeshMaterial3d(handles[part.material].clone()),
                        Transform::from_xyz(0.0, feet_offset, 0.0),
                    ));
                }
            });
        }
        None => {
            entity.with_children(|parent| {
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
