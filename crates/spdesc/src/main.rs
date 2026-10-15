//! `spdesc` — the SPDescendants client.
//!
//! Presentation only: the simulation lives in `spdesc-game-core` and is Bevy-free, and the asset
//! formats live in `spdesc-data`. The app resolves the read-only `GAME_DATA` folder through
//! `spdesc_game_core::game_data`, builds the Ultimate Bunny's meshes and textures from the
//! player's install at startup, and presents a movable character on a flat ground plane.

mod catalog;
mod combat;
mod content;
mod hair;
mod player;

use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};

use combat::{Weapon, age_effects, fire_weapon, retire_dead_enemies, spawn_enemies, tick_weapon};
use content::{CharacterAssets, DataSource, MaterialSpec};
use hair::{Hair, animate_hair};
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
        .add_systems(Startup, (setup_scene, grab_cursor, setup_character))
        .add_systems(
            Update,
            (
                menu_input,
                rebuild_model,
                draw_menu,
                (
                    toggle_cursor,
                    drive_player,
                    animate_locomotion,
                    animate_hair,
                    follow_camera,
                    tick_weapon,
                    fire_weapon,
                    age_effects,
                    retire_dead_enemies,
                )
                    .run_if(menu_closed)
                    .chain(),
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

/// Marks the character's model subtree, so a skin change despawns and rebuilds it in one go.
#[derive(Component)]
struct ModelRoot;

/// The character select screen's state.
#[derive(Resource, Default)]
struct Menu {
    characters: Vec<catalog::Character>,
    open: bool,
    /// 0 = character, 1 = body skin, 2 = head variant, 3 = face.
    column: usize,
    character: usize,
    skin: usize,
    head: usize,
    face: usize,
    /// Set when the player confirms; consumed by [`rebuild_model`].
    requested: bool,
}

/// The meshes and texture folders a selection resolves to.
struct Resolved {
    body: String,
    head: String,
    body_dir: String,
    head_dir: String,
    face_dir: Option<String>,
}

impl Menu {
    fn character(&self) -> Option<&catalog::Character> {
        self.characters.get(self.character)
    }

    fn skin(&self) -> Option<&catalog::Skin> {
        self.character().and_then(|c| c.skins.get(self.skin))
    }

    fn head(&self) -> Option<&catalog::Variant> {
        self.character().and_then(|c| c.heads.get(self.head))
    }

    fn face(&self) -> Option<&catalog::Variant> {
        self.character().and_then(|c| c.faces.get(self.face))
    }

    /// The length of each list, in column order.
    fn lengths(&self) -> [usize; 4] {
        [
            self.characters.len(),
            self.character().map_or(0, |c| c.skins.len()),
            self.character().map_or(0, |c| c.heads.len()),
            self.character().map_or(0, |c| c.faces.len()),
        ]
    }

    /// The current selection per column.
    fn selection(&self) -> [usize; 4] {
        [self.character, self.skin, self.head, self.face]
    }

    /// Move the selection in column `column` by one, wrapping.
    fn step(&mut self, column: usize, up: bool) {
        let length = self.lengths()[column];
        if length == 0 {
            return;
        }
        let selected: &mut usize = match column {
            0 => &mut self.character,
            1 => &mut self.skin,
            2 => &mut self.head,
            _ => &mut self.face,
        };
        *selected = if up {
            (*selected + length - 1) % length
        } else {
            (*selected + 1) % length
        };
    }

    /// Keep every selection inside its list (a shorter list after switching character).
    fn clamp(&mut self) {
        let lengths = self.lengths();
        let mut selection = self.selection();
        for (selected, length) in selection.iter_mut().zip(lengths) {
            *selected = if length == 0 {
                0
            } else {
                (*selected).min(length - 1)
            };
        }
        [self.character, self.skin, self.head, self.face] = selection;
    }

    /// The meshes and folders the current selection resolves to.
    fn resolved(&self) -> Option<Resolved> {
        let skin = self.skin()?;
        let head = self.head();
        Some(Resolved {
            body: skin.body.clone(),
            head: head
                .and_then(|v| v.mesh.clone())
                .unwrap_or_else(|| skin.head.clone()),
            body_dir: skin.material_dir.clone(),
            head_dir: head
                .map(|v| v.material_dir.clone())
                .unwrap_or_else(|| skin.material_dir.clone()),
            face_dir: self.face().map(|v| v.material_dir.clone()),
        })
    }
}

/// The spawned player and the root of its current model subtree.
#[derive(Resource, Default)]
struct Model {
    player: Option<Entity>,
    root: Option<Entity>,
}

/// The overlay text node the menu draws into.
#[derive(Component)]
struct MenuText;

/// Gameplay systems only run while the menu is closed.
fn menu_closed(menu: Res<Menu>) -> bool {
    !menu.open
}

/// Open the container and the local state.
fn open_data() -> anyhow::Result<DataSource> {
    let paks =
        spdesc_data::paks_dir().ok_or_else(|| anyhow::anyhow!("no GAME_DATA folder found"))?;
    let key = spdesc_data::key::aes_key()
        .ok_or_else(|| anyhow::anyhow!("no container key (spdesc-config/keys.json)"))?;
    let schema =
        spdesc_data::mapping::load().map_err(|e| anyhow::anyhow!("no property mapping: {e}"))?;
    DataSource::open(&paks, key, schema)
}

/// Build a resolved selection's meshes and textures.
fn build_character(resolved: &Resolved) -> anyhow::Result<CharacterAssets> {
    let mut data = open_data()?;
    content::load_character(
        &mut data,
        &resolved.body,
        &resolved.head,
        &resolved.body_dir,
        &resolved.head_dir,
        resolved.face_dir.as_deref(),
    )
}

/// The player entity, the initial model, and the menu overlay. Falls back to a capsule when the
/// data, key, or mapping is absent, so the program is still playable.
fn setup_character(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inverse_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    let player = commands
        .spawn((
            Player::default(),
            Weapon::default(),
            Transform::default(),
            Visibility::default(),
        ))
        .id();
    let mut model = Model {
        player: Some(player),
        root: None,
    };
    let mut menu = Menu::default();
    match open_data().map(|data| catalog::build(&data.index, &catalog::load_names())) {
        Ok(characters) => menu.characters = characters,
        Err(error) => warn!("could not enumerate characters ({error:#})"),
    }
    // Default to Bunny's `CMN 001`, the skin this checkout started with.
    if let Some(i) = menu.characters.iter().position(|c| c.id == "004") {
        menu.character = i;
        if let Some(j) = menu.characters[i]
            .skins
            .iter()
            .position(|s| s.label == "CMN 001")
        {
            menu.skin = j;
        }
    }
    info!(
        "catalog: {} characters, e.g. {}",
        menu.characters.len(),
        menu.characters
            .iter()
            .map(|c| format!("{} {}", c.id, c.name))
            .collect::<Vec<_>>()
            .join(", ")
    );

    match menu.resolved().map(|r| build_character(&r)) {
        Some(Ok(assets)) => {
            model.root = Some(spawn_model(
                &mut commands,
                &mut meshes,
                &mut images,
                &mut materials,
                &mut inverse_bindposes,
                player,
                assets,
            ));
        }
        Some(Err(error)) => warn!("could not build the character ({error:#}); using a capsule"),
        None => {}
    }
    if model.root.is_none() {
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
    commands.insert_resource(model);
    commands.insert_resource(menu);

    commands.spawn((
        MenuText,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgb(0.96, 0.96, 0.98)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(18.0),
            top: Val::Px(14.0),
            ..default()
        },
    ));
}

/// Spawn a skin's model subtree under a fresh [`ModelRoot`] on `player`, and return the root.
#[allow(clippy::too_many_arguments)]
fn spawn_model(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    inverse_bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
    player: Entity,
    assets: CharacterAssets,
) -> Entity {
    let CharacterAssets {
        parts,
        materials: specs,
        skeleton,
        head_skeleton,
        head_attach,
        face_attach,
        feet_offset,
        animation,
    } = assets;
    let root = commands
        .spawn((
            ModelRoot,
            Transform::default(),
            Visibility::default(),
            ChildOf(player),
        ))
        .id();
    info!(
        "character: {} section mesh(es), {} material(s), {} bones, feet offset {feet_offset:.3} m",
        parts.len(),
        specs.len(),
        skeleton.names.len()
    );
    let handles: Vec<_> = specs
        .into_iter()
        .map(|spec| materials.add(build_material(images, spec)))
        .collect();
    let mut body_parts = Vec::new();
    let mut head_parts = Vec::new();
    let mut face_parts = Vec::new();
    for part in parts {
        match part.attach {
            content::Attach::Body => body_parts.push(part),
            content::Attach::Head => head_parts.push(part),
            content::Attach::Face => face_parts.push(part),
        }
    }

    // One entity per bone, parented into the hierarchy under the model root, so the skinned mesh's
    // joint matrices carry the player's world transform. The `Animator` poses them from the clips.
    let mut bones: Vec<Entity> = Vec::with_capacity(skeleton.names.len());
    for local in &skeleton.locals {
        bones.push(commands.spawn((*local, Visibility::default())).id());
    }
    for (i, parent) in skeleton.parents.iter().enumerate() {
        let parent_entity = if *parent >= 0 {
            bones[*parent as usize]
        } else {
            root
        };
        commands.entity(bones[i]).insert(ChildOf(parent_entity));
    }
    let inverse = inverse_bindposes.add(SkinnedMeshInverseBindposes::from(
        skeleton.inverse_bindposes.clone(),
    ));

    // A stand-in weapon on the right hand bone, so the character visibly carries a gun. A real
    // weapon mesh from GAME_DATA is a later slice (its own mesh and materials).
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
            air: animation.air,
            land: animation.land,
            bone_to_track: animation.bone_to_track,
            bones: bones.clone(),
            skeleton,
            time: 0.0,
            air_timer: 0.0,
            land_timer: 0.0,
            was_grounded: true,
            active: animation.idle,
            active_at: 0.0,
            active_looping: true,
            prev: animation.idle,
            prev_at: 0.0,
            prev_looping: true,
            blend: 1.0,
        });
    }

    // The body: skinned sub-meshes on the root. The head: skinned sub-meshes on the head attach
    // node, so they follow the head bone and the braid dynamics.
    commands.entity(root).with_children(|parent| {
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
        // The head's own skeleton lives on the attach node's frame, so the head skinning and the
        // braid dynamics share one root — mirroring how the body's bones share the model root's.
        let mut head_bones: Vec<Entity> = Vec::with_capacity(head_skeleton.names.len());
        for bone_local in &head_skeleton.locals {
            head_bones.push(commands.spawn((*bone_local, Visibility::default())).id());
        }
        for (i, parent) in head_skeleton.parents.iter().enumerate() {
            let parent_entity = if *parent >= 0 {
                head_bones[*parent as usize]
            } else {
                node
            };
            commands
                .entity(head_bones[i])
                .insert(ChildOf(parent_entity));
        }
        let head_inverse = inverse_bindposes.add(SkinnedMeshInverseBindposes::from(
            head_skeleton.inverse_bindposes.clone(),
        ));
        commands.entity(node).with_children(|parent| {
            for part in head_parts {
                parent.spawn((
                    Mesh3d(meshes.add(part.mesh)),
                    MeshMaterial3d(handles[part.material].clone()),
                    Transform::default(),
                    SkinnedMesh {
                        inverse_bindposes: head_inverse.clone(),
                        joints: head_bones.clone(),
                    },
                ));
            }
        });
        commands
            .entity(node)
            .insert(Hair::new(&head_skeleton, head_bones));
    }
    // The separate face mesh, static on its own attach node (which follows the head bone).
    if let Some((index, local)) = face_attach {
        let node = commands
            .spawn((local, Visibility::default(), ChildOf(bones[index])))
            .id();
        commands.entity(node).with_children(|parent| {
            for part in face_parts {
                parent.spawn((
                    Mesh3d(meshes.add(part.mesh)),
                    MeshMaterial3d(handles[part.material].clone()),
                    Transform::default(),
                ));
            }
        });
    }
    root
}

/// Set the cursor's grab state, so the pointer is free while the menu is open.
fn set_cursor_grab(grabbed: bool, windows: &mut Query<&mut CursorOptions, With<PrimaryWindow>>) {
    for mut options in windows {
        options.grab_mode = if grabbed {
            CursorGrabMode::Locked
        } else {
            CursorGrabMode::None
        };
        options.visible = !grabbed;
    }
}

/// Tab opens and closes the menu; while it is open the arrows move and Enter applies.
fn menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: ResMut<Menu>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        menu.open = !menu.open;
        set_cursor_grab(!menu.open, &mut windows);
        return;
    }
    if !menu.open {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        menu.open = false;
        set_cursor_grab(true, &mut windows);
        return;
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        menu.column = (menu.column + 3) % 4;
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        menu.column = (menu.column + 1) % 4;
    }
    if menu.characters.is_empty() {
        return;
    }
    let up = keys.just_pressed(KeyCode::ArrowUp);
    let down = keys.just_pressed(KeyCode::ArrowDown);
    if up || down {
        let column = menu.column;
        menu.step(column, up);
    }
    menu.clamp();
    if keys.just_pressed(KeyCode::Enter) && menu.skin().is_some() {
        menu.requested = true;
        menu.open = false;
        set_cursor_grab(true, &mut windows);
    }
}

/// Rebuild the model when a new skin is confirmed.
fn rebuild_model(
    mut commands: Commands,
    mut menu: ResMut<Menu>,
    mut model: ResMut<Model>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inverse_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    if !menu.requested {
        return;
    }
    menu.requested = false;
    let (Some(player), Some(resolved)) = (model.player, menu.resolved()) else {
        return;
    };
    if let Some(root) = model.root.take() {
        commands.entity(root).despawn();
    }
    commands.entity(player).remove::<Animator>();
    let name = menu.character().map(|c| c.name.clone()).unwrap_or_default();
    info!(
        "skin: {name} {}",
        menu.skin().map(|s| s.label.clone()).unwrap_or_default()
    );
    match build_character(&resolved) {
        Ok(assets) => {
            model.root = Some(spawn_model(
                &mut commands,
                &mut meshes,
                &mut images,
                &mut materials,
                &mut inverse_bindposes,
                player,
                assets,
            ));
        }
        Err(error) => warn!("could not load {} ({error:#})", resolved.body),
    }
}

/// Draw the menu overlay: characters, body skins, head variants and faces, one column per list.
fn draw_menu(menu: Res<Menu>, mut texts: Query<&mut Text, With<MenuText>>) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    if !menu.open {
        if !text.0.is_empty() {
            text.0.clear();
        }
        return;
    }
    let column = |index: usize, values: Vec<String>| -> Vec<String> {
        values
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                format!(
                    "{} {}",
                    if menu.column == index && i == menu.selection()[index] {
                        '>'
                    } else {
                        ' '
                    },
                    label
                )
            })
            .collect()
    };
    let characters = column(
        0,
        menu.characters
            .iter()
            .map(|c| format!("{:>3} {}", c.id, c.name))
            .collect(),
    );
    let (skins, heads, faces) = match menu.character() {
        Some(c) => (
            column(1, c.skins.iter().map(|s| s.label.clone()).collect()),
            column(2, c.heads.iter().map(|h| h.label.clone()).collect()),
            column(3, c.faces.iter().map(|f| f.label.clone()).collect()),
        ),
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    let rows = characters
        .len()
        .max(skins.len())
        .max(heads.len())
        .max(faces.len());
    let mut lines = vec![format!(
        "{:<16}{:<14}{:<16}{}",
        "CHARACTERS", "BODY SKIN", "HEAD", "FACE"
    )];
    for i in 0..rows {
        let get = |v: &Vec<String>| v.get(i).cloned().unwrap_or_default();
        lines.push(format!(
            "{:<16}{:<14}{:<16}{}",
            get(&characters),
            get(&skins),
            get(&heads),
            get(&faces)
        ));
    }
    lines.push(String::new());
    lines.push("Tab close    left/right column    up/down select    Enter apply".into());
    let rendered = lines.join("\n");
    if text.0 != rendered {
        text.0 = rendered;
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
