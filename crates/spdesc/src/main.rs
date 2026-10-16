//! `spdesc` — the SPDescendants client.
//!
//! Presentation only: the simulation lives in `spdesc-game-core` and is Bevy-free, and the asset
//! formats live in `spdesc-data`. The app resolves the read-only `TFD_DATA` folder through
//! `spdesc_game_core::tfd_data`, builds the Ultimate Bunny's meshes and textures from the
//! player's install at startup, and presents a movable character on a flat ground plane.

mod audio;
mod catalog;
mod combat;
mod content;
mod hair;
mod loading;
mod map;
mod menu;
mod player;
mod progress;
mod settings;

use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};

use map::{
    ChosenMap, MapCollision, MapMeshes, Previews, build_rows, collect_map_mesh, collect_previews,
    follow_choice, show_dev_arena, sync_map_mesh, sync_targets,
};
use menu::{
    draw_dev, draw_map, draw_pages, draw_rows, draw_scrollbars, draw_tabs, menu_keys, menu_pointer,
    place_menu, setup_menu,
};
use progress::{
    award_experience, debug_progress, draw_progress, save_career, save_career_on_exit,
    setup_progress,
};
use settings::{CareerSave, Settings, apply_volumes};
use spdesc_mw2::SoundBank;

use audio::Sounds;
use combat::{Muzzle, Weapon, WeaponRig, age_effects, fire_weapon, tick_weapon, update_targets};
use content::{CharacterAssets, MaterialSpec};
use hair::{Hair, animate_hair};
use loading::{ModelKey, ModelLoader};
use player::{
    Animator, CameraRig, Player, animate_locomotion, drive_player, follow_camera, toggle_cursor,
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
        // The menu's open flag is read by a run condition, which runs before any startup system
        // does: it has to exist from the first frame.
        .init_resource::<menu::Menu>()
        .init_resource::<ChosenMap>()
        .init_resource::<MapCollision>()
        .init_resource::<MapMeshes>()
        .add_systems(
            Startup,
            (
                setup_scene,
                setup_hud,
                setup_progress,
                setup_menu,
                setup_settings,
                start_loader,
                setup_mw2,
                setup_character,
            )
                // Ordered, so each system's resources are in place for the next: the menus need
                // the settings, and the character the sound bank the map catalogue comes from.
                .chain(),
        )
        .add_systems(
            Update,
            (
                (
                    menu_keys,
                    menu_pointer,
                    collect_previews,
                    follow_choice,
                    sync_map_mesh,
                    collect_map_mesh,
                    show_dev_arena,
                    sync_targets,
                    draw_map,
                    draw_map_label,
                    stream_models,
                    apply_model,
                    save_selection,
                )
                    .chain(),
                (
                    place_menu,
                    draw_pages,
                    draw_tabs,
                    draw_rows,
                    draw_scrollbars,
                    draw_dev,
                    draw_hud,
                    draw_progress,
                    award_experience,
                    debug_progress,
                    apply_volumes,
                    save_career,
                    save_career_on_exit,
                )
                    .chain(),
                (
                    toggle_cursor,
                    drive_player,
                    animate_locomotion,
                    animate_hair,
                    follow_camera,
                    tick_weapon,
                    fire_weapon,
                    age_effects,
                    update_targets,
                )
                    .run_if(menu_closed)
                    .chain(),
            )
                .chain(),
        )
        .run();
}

/// Read the Modern Warfare 2 install once and build everything that comes out of it: the sound
/// cues, the rank table, and the insignia art. Each piece stands on its own — with no install the
/// cues are synthesised, the career stays empty and the HUD hides.
fn setup_mw2(
    mut commands: Commands,
    mut sources: ResMut<Assets<AudioSource>>,
    mut images: ResMut<Assets<Image>>,
) {
    let started = std::time::Instant::now();
    let install = spdesc_game_core::mw2_data();
    let bank = install
        .as_deref()
        .map(|install| SoundBank::read(install, &spdesc_mw2::ZONES));
    match &bank {
        Some(bank) if bank.alias_count() > 0 => {
            info!(
                "sound data: {} aliases and {} table(s) from {} zone(s) of {} in {:.1} s",
                bank.alias_count(),
                bank.zones()
                    .iter()
                    .map(|(_, data)| data.tables.len())
                    .sum::<usize>(),
                bank.zones().len(),
                install
                    .as_deref()
                    .unwrap_or(std::path::Path::new("?"))
                    .display(),
                started.elapsed().as_secs_f32()
            );
            for (zone, error) in bank.failures() {
                warn!("sound data: {zone}.ff could not be read — {error}");
            }
        }
        Some(bank) => warn!(
            "sound data: {} holds no readable data — {:#?}",
            install
                .as_deref()
                .unwrap_or(std::path::Path::new("?"))
                .display(),
            bank.failures()
        ),
        None => info!("sound data: no MW2_DATA folder found — using the synthesised cues"),
    }

    let mut career =
        progress::Career::from_zones(bank.as_ref().map(|bank| bank.zones()).unwrap_or(&[]));
    // A saved career is taken up before anything reads it, so the HUD and the dev menu show the
    // level the player left.
    let saved = CareerSave::load();
    career.restore(saved);
    if let Some(saved) = saved
        && career.is_loaded()
    {
        info!(
            "progression: resumed at level {} (rank {}), prestige {}, {} xp in rank",
            career.rank().map(|r| r.index + 1).unwrap_or(0),
            career.rank().map(|r| r.title.clone()).unwrap_or_default(),
            career.progression().prestige(),
            saved.xp_in_rank,
        );
    }
    if career.is_loaded() {
        let ranks = career.ranks();
        info!(
            "progression: {} ranks to {}, {prestige} prestige levels available",
            ranks.len(),
            ranks.last_index().map(|last| last + 1).unwrap_or(0),
            prestige = ranks.max_prestige()
        );
    } else {
        info!("progression: no rank table — the career and its HUD stay hidden");
    }
    let insignia = progress::load_insignia(install.as_deref(), &career, &mut images);

    commands.insert_resource(Sounds::load(&mut sources, bank.as_ref()));
    // The bank is kept: the map catalogue is read out of the same zone, and an install that is
    // absent leaves an empty bank rather than no bank at all.
    commands.insert_resource(Zones(bank.unwrap_or_default()));
    commands.insert_resource(career);
    commands.insert_resource(insignia);
}

/// Start the thread that builds character models, so a preview never blocks the frame.
fn start_loader(mut commands: Commands) {
    commands.insert_resource(ModelLoader::start());
}

/// Load the sound levels and report them, so a run says which ones it is using.
fn setup_settings(mut commands: Commands) {
    let settings = Settings::load();
    info!(
        "settings: master {:.0}%, gun fire {:.0}%",
        settings.master * 100.0,
        settings.gun_fire * 100.0
    );
    commands.insert_resource(settings);
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

    // A large checker ground plane at Y = 0 (200 m square), the "basic map of any kind". It is
    // the project's own arena, so an imported map hides it rather than drawing both.
    commands.spawn((
        DevArena,
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
            DevArena,
            Mesh3d(pillar.clone()),
            MeshMaterial3d(pillar_material.clone()),
            Transform::from_xyz(x, 2.0, z),
        ));
    }

    // The practice targets are not part of the ground: they belong to the map being played, and
    // `map::sync_targets` puts them up when that map is the project's own arena.
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

/// The spawned player and the root of its current model subtree.
#[derive(Resource, Default)]
struct Model {
    player: Option<Entity>,
    root: Option<Entity>,
    /// The model those meshes were built from, so the loader is not asked for it again.
    shown: Option<ModelKey>,
}

/// The overlay text node the HUD (ammunition, reload) draws into.
#[derive(Component)]
struct HudText;

/// The crosshair's reticle; hidden while the character menu is open.
#[derive(Component)]
struct Reticle;

/// The line naming the map the game is playing on.
#[derive(Component)]
struct MapLabel;

/// The project's own arena: its ground and its reference pillars, which an imported map hides.
#[derive(Component)]
pub(crate) struct DevArena;

/// The Modern Warfare 2 zones the app read, kept so the things built out of them — the map
/// catalogue — are built from what the install actually ships. `SoundBank` itself is Bevy-free,
/// which is why it is wrapped rather than made a resource there.
#[derive(Resource, Default)]
struct Zones(SoundBank);

/// The crosshair and the ammunition readout.
fn setup_hud(mut commands: Commands) {
    commands
        .spawn((
            Reticle,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|parent| {
            parent.spawn((
                Node {
                    width: Val::Px(16.0),
                    height: Val::Px(16.0),
                    border: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                BorderColor::from(Color::srgba(1.0, 1.0, 1.0, 0.85)),
            ));
        });
    commands.spawn((
        MapLabel,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgb(0.86, 0.9, 0.96)),
        TextLayout::justify(bevy::text::Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(18.0),
            ..default()
        },
    ));
    commands.spawn((
        HudText,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::srgb(0.96, 0.96, 0.98)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(28.0),
            bottom: Val::Px(22.0),
            ..default()
        },
    ));
}

/// Name the map being played, so the choice is visible outside the menu as well as inside it.
fn draw_map_label(
    menu: Res<menu::Menu>,
    chosen: Res<ChosenMap>,
    mut labels: Query<&mut Text, With<MapLabel>>,
) {
    let Ok(mut text) = labels.single_mut() else {
        return;
    };
    let rendered = if menu.open || chosen.title.is_empty() {
        String::new()
    } else {
        chosen.title.clone()
    };
    if text.0 != rendered {
        text.0 = rendered;
    }
}

/// Draw the HUD: the crosshair over the aim, and the magazine and reload countdown.
fn draw_hud(
    menu: Res<menu::Menu>,
    weapons: Query<&Weapon>,
    mut texts: Query<&mut Text, With<HudText>>,
    mut reticles: Query<&mut Visibility, With<Reticle>>,
) {
    let visibility = if menu.open {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut reticle in &mut reticles {
        *reticle = visibility;
    }
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    if menu.open {
        if !text.0.is_empty() {
            text.0.clear();
        }
        return;
    }
    let Ok(weapon) = weapons.single() else {
        return;
    };
    let rendered = if weapon.combat.reload_remaining > 0.0 {
        format!(
            "RELOADING  {:.1}s\n{} / {}",
            weapon.combat.reload_remaining, weapon.combat.ammo, weapon.params.magazine
        )
    } else {
        format!("{} / {}", weapon.combat.ammo, weapon.params.magazine)
    };
    if text.0 != rendered {
        text.0 = rendered;
    }
}

/// Gameplay systems only run while the menu is closed: it holds the game and frees the cursor.
fn menu_closed(menu: Res<menu::Menu>) -> bool {
    !menu.open
}

/// Open the container and the local state.
fn build_character(key: &ModelKey) -> anyhow::Result<CharacterAssets> {
    let mut data = content::open_data()?;
    key.load(&mut data)
}

/// The player entity, the initial model, and the menu overlay. Falls back to a capsule when the
/// data, key, or mapping is absent, so the program is still playable.
#[allow(clippy::too_many_arguments)]
fn setup_character(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inverse_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    bank: Res<Zones>,
    settings: Res<Settings>,
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
        shown: None,
    };
    let mut menu = menu::Menu::default();
    let mut catalog = Vec::new();
    match content::open_data().map(|data| catalog::build(&data.index, &catalog::load_names())) {
        Ok(characters) => catalog = characters,
        Err(error) => warn!("could not enumerate characters ({error:#})"),
    }
    let listed = catalog.len();
    let names = catalog
        .iter()
        .map(|c| format!("{} {}", c.id, c.name))
        .collect::<Vec<_>>()
        .join(", ");
    menu.fill(catalog);
    // The last applied selection wins over the default, so a choice survives a restart.
    if let Some(saved) = settings::Selection::load() {
        menu.restore(&saved);
        info!("selection: {}", menu.saved().describe());
    }
    info!("catalog: {listed} characters, e.g. {names}");

    // The maps the install ships, and the one the player last chose. The game opens on the map
    // page, because the ground to play on is the first thing a game asks for.
    let rows = build_rows(&bank.0, spdesc_game_core::mw2_data().as_deref());
    info!(
        "maps: {} offered, e.g. {}",
        rows.len(),
        rows.iter()
            .take(6)
            .map(|row| row.map.title.clone())
            .collect::<Vec<_>>()
            .join(", ")
    );
    commands.insert_resource(Previews::start(&rows));
    menu.fill_maps(rows, &settings.map);
    menu.open = true;

    let initial = menu.key();
    match initial.as_ref().map(build_character) {
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
            model.shown = initial;
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
        face,
        weapon,
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
            // The weapon's parts arrive with the weapon, not in the character's `parts`.
            content::Attach::Weapon => {}
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

    // The equipped weapon rides a socket node on the right hand: the node carries the recoil kick,
    // its sub-meshes hang off it, and a muzzle marker at the barrel's tip feeds the tracers.
    if let Some(weapon) = weapon {
        let weapon_handles: Vec<_> = weapon
            .materials
            .into_iter()
            .map(|spec| materials.add(build_material(images, spec)))
            .collect();
        if let Some((socket, local)) = weapon.attach {
            let rig = commands
                .spawn((
                    WeaponRig::default(),
                    local,
                    Visibility::default(),
                    ChildOf(bones[socket]),
                ))
                .id();
            commands.entity(rig).with_children(|parent| {
                for part in weapon.parts {
                    parent.spawn((
                        Mesh3d(meshes.add(part.mesh)),
                        MeshMaterial3d(weapon_handles[part.material].clone()),
                        Transform::default(),
                    ));
                }
                parent.spawn((
                    Muzzle,
                    Transform::from_translation(weapon.muzzle),
                    Visibility::default(),
                ));
            });
        }
    } else if let Some(hand) = skeleton.names.iter().position(|n| n == "Bip001-R-Hand") {
        // No weapon data: a stand-in block so the character still visibly carries something.
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
        let start = animation.armed.unwrap_or(animation.base).idle;
        commands.entity(player).insert(Animator {
            clips: animation.clips,
            bone_to_track: animation.bone_to_track,
            base: animation.base,
            armed: animation.armed,
            aim: animation.aim,
            reload: animation.reload,
            upper: player::upper_body_mask(&skeleton),
            upper_root: player::upper_body_root(&skeleton),
            weapon_socket: skeleton
                .names
                .iter()
                .position(|name| name == content::WEAPON_SOCKET),
            ads: 0.0,
            aim_time: 0.0,
            step_timer: 0.0,
            bones: bones.clone(),
            skeleton,
            time: 0.0,
            air_timer: 0.0,
            land_timer: 0.0,
            was_grounded: true,
            active: start,
            active_at: 0.0,
            active_looping: true,
            prev: start,
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
    // The separate face mesh: a head/neck/chest bust skinned to the *body* skeleton, so it sits in
    // body model space with the body parts and its neck follows the neck and spine bones.
    if let Some(face) = face {
        let face_inverse =
            inverse_bindposes.add(SkinnedMeshInverseBindposes::from(face.inverse_bindposes));
        let face_joints: Vec<Entity> = face.joints.iter().map(|&bone| bones[bone]).collect();
        commands.entity(root).with_children(|parent| {
            for part in face_parts {
                parent.spawn((
                    Mesh3d(meshes.add(part.mesh)),
                    MeshMaterial3d(handles[part.material].clone()),
                    Transform::from_xyz(0.0, feet_offset, 0.0),
                    SkinnedMesh {
                        inverse_bindposes: face_inverse.clone(),
                        joints: face_joints.clone(),
                    },
                ));
            }
        });
    }
    root
}

/// Set the cursor's grab state, so the pointer is free while the menu is open.
pub(crate) fn set_cursor_grab(
    grabbed: bool,
    windows: &mut Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    for mut options in windows {
        options.grab_mode = if grabbed {
            CursorGrabMode::Locked
        } else {
            CursorGrabMode::None
        };
        options.visible = !grabbed;
    }
}

/// Say what the app wants the worker to build: the model the highlight resolves to, and — while the
/// menu is open — the ones a step away, so a preview never waits for a cold build.
fn stream_models(mut loader: ResMut<ModelLoader>, menu: Res<menu::Menu>, model: Res<Model>) {
    loader.collect();
    if model.player.is_none() {
        return;
    }
    let shown = model.shown.as_ref();
    if let Some(key) = menu.key()
        && shown != Some(&key)
    {
        loader.request(key, false);
    }
    if menu.open {
        for key in menu.prefetch_keys() {
            if shown != Some(&key) {
                loader.request(key, true);
            }
        }
    }
}

/// Put the model the highlight resolves to on the character, once the worker has built it. The
/// model on screen stays until its replacement is ready, so a preview never blinks to nothing.
fn apply_model(
    mut commands: Commands,
    menu: Res<menu::Menu>,
    mut model: ResMut<Model>,
    mut loader: ResMut<ModelLoader>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut inverse_bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    let Some(key) = menu.key() else {
        return;
    };
    if model.shown.as_ref() == Some(&key) {
        return;
    }
    let (Some(player), Some(_)) = (model.player, model.root) else {
        return;
    };
    let Some(assets) = loader.take(&key) else {
        return;
    };
    // A new model replaces the old one whole, so the animator goes with it and the loader rebuilds
    // it from the new clips.
    if let Some(root) = model.root.take() {
        commands.entity(root).despawn();
    }
    commands.entity(player).remove::<Animator>();
    info!("model: showing {}", key.label());
    model.root = Some(spawn_model(
        &mut commands,
        &mut meshes,
        &mut images,
        &mut materials,
        &mut inverse_bindposes,
        player,
        assets,
    ));
    model.shown = Some(key);
}

/// Remember the selection when the player confirms it. The highlight is previewed freely; only a
/// confirmation is written, so a walk through the list does not rewrite the file at every step.
fn save_selection(mut menu: ResMut<menu::Menu>, mut settings: ResMut<Settings>) {
    if !menu.requested {
        return;
    }
    menu.requested = false;
    if let Err(error) = menu.saved().save() {
        warn!("could not save the selection ({error:#})");
    }
    // The map is a setting, so it is written with the settings — and only when it has changed, so
    // a walk through the map list does not rewrite the file at every step.
    let map = menu.saved_map();
    if settings.map != map {
        settings.map = map;
        if let Err(error) = settings.save() {
            warn!("could not save the map choice ({error:#})");
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
