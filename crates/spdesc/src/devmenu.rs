//! The developer menu: a panel over the game for the things that are not gameplay — the level and
//! prestige, and the sound levels.
//!
//! It is the game's own Bevy UI, not a second UI stack: `bevy_egui` needs Bevy 0.18 and this tree
//! is on 0.20, so the panel, its buttons and its sliders are nodes like the character menu's. Input
//! is hit-tested against [`CONTROLS`], the one table that decides both what is drawn and what is
//! clicked, so the two cannot drift apart.
//!
//! `F1` opens and closes it. While it is open the cursor is free and gameplay is held, exactly as
//! the character menu does.

use bevy::audio::Volume;
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::progress::{self, Career};
use crate::set_cursor_grab;
use crate::settings::Settings;

/// Where the panel sits, in logical pixels from the window's top-left, and how big it is.
const PANEL: Vec2 = Vec2::new(40.0, 56.0);
const PANEL_SIZE: Vec2 = Vec2::new(304.0, 396.0);

/// What a control does when it is used.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Grant,
    Prestige,
    ResetPrestige,
    GunFire,
    Master,
    Close,
}

/// Whether a control is pressed or dragged.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Button,
    Slider,
}

/// One control: what it does, where it is inside the panel (`x`, `y`, width, height), and its
/// label.
struct Control {
    action: Action,
    kind: Kind,
    rect: [f32; 4],
    label: &'static str,
}

const CONTROLS: &[Control] = &[
    Control {
        action: Action::Grant,
        kind: Kind::Button,
        rect: [16.0, 96.0, 272.0, 28.0],
        label: "Grant 25 000 XP",
    },
    Control {
        action: Action::Prestige,
        kind: Kind::Button,
        rect: [16.0, 130.0, 272.0, 28.0],
        label: "Prestige now",
    },
    Control {
        action: Action::ResetPrestige,
        kind: Kind::Button,
        rect: [16.0, 164.0, 272.0, 28.0],
        label: "Reset prestige",
    },
    Control {
        action: Action::GunFire,
        kind: Kind::Slider,
        rect: [16.0, 226.0, 272.0, 14.0],
        label: "Gun fire volume",
    },
    Control {
        action: Action::Master,
        kind: Kind::Slider,
        rect: [16.0, 298.0, 272.0, 14.0],
        label: "Master volume",
    },
    Control {
        action: Action::Close,
        kind: Kind::Button,
        rect: [16.0, 346.0, 272.0, 28.0],
        label: "Close",
    },
];

/// Whether the panel is open.
#[derive(Resource, Default)]
pub(crate) struct DevMenu {
    pub(crate) open: bool,
}

/// The panel root, hidden while the menu is closed.
#[derive(Component)]
pub(crate) struct DevPanel;

/// The line naming the career the panel is acting on.
#[derive(Component)]
pub(crate) struct DevStatus;

/// A text node the panel never changes after it is built.
#[derive(Component)]
pub(crate) struct FixedText;

/// The filled part of a slider's track, and the text showing its value.
#[derive(Component)]
pub(crate) struct SliderFill(Action);

#[derive(Component)]
pub(crate) struct SliderValue(Action);

/// Build the panel once, hidden.
pub(crate) fn setup_dev_menu(mut commands: Commands) {
    commands
        .spawn((
            DevPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(PANEL.x),
                top: Val::Px(PANEL.y),
                width: Val::Px(PANEL_SIZE.x),
                height: Val::Px(PANEL_SIZE.y),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.06, 0.09, 0.94)),
            BorderColor::from(Color::srgba(0.65, 0.72, 0.85, 0.55)),
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            absolute_text(
                panel,
                [16.0, 14.0, 272.0, 20.0],
                "DEVELOPER MENU   (F1)",
                15.0,
                Color::srgb(0.98, 0.86, 0.5),
                FixedText,
            );
            absolute_text(
                panel,
                [16.0, 42.0, 272.0, 44.0],
                "",
                14.0,
                Color::srgb(0.9, 0.92, 0.96),
                DevStatus,
            );
            for control in CONTROLS {
                let [x, y, w, h] = control.rect;
                match control.kind {
                    Kind::Button => {
                        panel
                            .spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(x),
                                    top: Val::Px(y),
                                    width: Val::Px(w),
                                    height: Val::Px(h),
                                    border: UiRect::all(Val::Px(1.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.16, 0.19, 0.26)),
                                BorderColor::from(Color::srgba(0.6, 0.68, 0.82, 0.5)),
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new(control.label),
                                    TextFont {
                                        font_size: bevy::text::FontSize::Px(14.0),
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.94, 0.95, 0.98)),
                                ));
                            });
                    }
                    Kind::Slider => {
                        absolute_text(
                            panel,
                            [x, y - 22.0, w - 70.0, 18.0],
                            control.label,
                            14.0,
                            Color::srgb(0.88, 0.9, 0.95),
                            FixedText,
                        );
                        absolute_text(
                            panel,
                            [x + w - 66.0, y - 22.0, 66.0, 18.0],
                            "",
                            14.0,
                            Color::srgb(0.98, 0.86, 0.5),
                            SliderValue(control.action),
                        );
                        panel
                            .spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(x),
                                    top: Val::Px(y),
                                    width: Val::Px(w),
                                    height: Val::Px(h),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.16)),
                            ))
                            .with_children(|track| {
                                track.spawn((
                                    SliderFill(control.action),
                                    Node {
                                        width: Val::Percent(100.0),
                                        height: Val::Percent(100.0),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.86, 0.78, 0.42)),
                                ));
                            });
                    }
                }
            }
        });
}

/// A text node at an absolute spot inside the panel, with a marker.
fn absolute_text<M: Component>(
    panel: &mut ChildSpawnerCommands,
    rect: [f32; 4],
    body: &str,
    size: f32,
    colour: Color,
    marker: M,
) {
    let [x, y, w, h] = rect;
    panel.spawn((
        marker,
        Text::new(body),
        TextFont {
            font_size: bevy::text::FontSize::Px(size),
            ..default()
        },
        TextColor(colour),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x),
            top: Val::Px(y),
            width: Val::Px(w),
            height: Val::Px(h),
            ..default()
        },
    ));
}

/// `F1` opens the panel, a press on a control uses it, and a press or drag on a slider sets it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dev_menu_input(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut dev: ResMut<DevMenu>,
    mut settings: ResMut<Settings>,
    mut career: ResMut<Career>,
    mut unsaved: Local<bool>,
) {
    if keys.just_pressed(KeyCode::F1) {
        dev.open = !dev.open;
        set_cursor_grab(!dev.open, &mut cursor);
        return;
    }
    if !dev.open {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        dev.open = false;
        set_cursor_grab(true, &mut cursor);
        return;
    }

    let pressed = buttons.pressed(MouseButton::Left);
    let clicked = buttons.just_pressed(MouseButton::Left);
    let released = buttons.just_released(MouseButton::Left);
    if let Ok(window) = windows.single()
        && let Some(at) = window.cursor_position()
    {
        let at = at - PANEL;
        for control in CONTROLS {
            if !hit(control.rect, at) {
                continue;
            }
            match control.kind {
                Kind::Slider if pressed => {
                    let value = share(control.rect, at);
                    let settings = settings.as_mut();
                    match control.action {
                        Action::GunFire => settings.gun_fire = value,
                        Action::Master => settings.master = value,
                        _ => {}
                    }
                    *unsaved = true;
                }
                Kind::Button if clicked => match control.action {
                    Action::Grant => progress::grant(&mut career, GRANT),
                    Action::Prestige => {
                        let prestiged = career.prestige();
                        if !prestiged {
                            info!("dev menu: prestige is not on offer");
                        }
                    }
                    Action::ResetPrestige => {
                        career.reset();
                        info!("dev menu: prestige reset");
                    }
                    Action::Close => {
                        dev.open = false;
                        set_cursor_grab(true, &mut cursor);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    // The settings are written when the drag ends, not on every frame of it.
    if *unsaved && (released || !pressed) {
        if let Err(error) = settings.save() {
            warn!("dev menu: could not write the settings: {error}");
        }
        *unsaved = false;
    }
}

/// What the panel's "grant" button awards.
const GRANT: u32 = 25_000;

/// Whether `at`, in panel-relative pixels, is inside a control's rect.
fn hit(rect: [f32; 4], at: Vec2) -> bool {
    at.x >= rect[0] && at.x <= rect[0] + rect[2] && at.y >= rect[1] && at.y <= rect[1] + rect[3]
}

/// The share of a track a point sits at: a slider's value, clamped to its ends.
fn share(rect: [f32; 4], at: Vec2) -> f32 {
    ((at.x - rect[0]) / rect[2]).clamp(0.0, 1.0)
}

/// Show or hide the panel, and keep its readouts current.
pub(crate) fn draw_dev_menu(
    dev: Res<DevMenu>,
    career: Res<Career>,
    settings: Res<Settings>,
    mut panels: Query<&mut Visibility, With<DevPanel>>,
    mut status: Query<&mut Text, (With<DevStatus>, Without<SliderValue>)>,
    mut values: Query<(&mut Text, &SliderValue)>,
    mut fills: Query<(&mut Node, &SliderFill)>,
) {
    let wanted = if dev.open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut panels {
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
    if !dev.open {
        return;
    }
    if let Ok(mut text) = status.single_mut() {
        let rendered = if career.is_loaded() {
            let rank = career.rank();
            format!(
                "level {}   rank {}   prestige {}\nxp in rank {}",
                rank.map(|r| r.index + 1).unwrap_or(0),
                rank.map(|r| r.title.clone()).unwrap_or_default(),
                career.progression().prestige(),
                career.progression().xp_in_rank()
            )
        } else {
            "no rank table: the career and its HUD stay hidden".to_owned()
        };
        if text.0 != rendered {
            text.0 = rendered;
        }
    }
    for (mut text, which) in &mut values {
        let value = match which.0 {
            Action::GunFire => settings.gun_fire,
            Action::Master => settings.master,
            _ => 0.0,
        };
        let rendered = format!("{:.0}%", value * 100.0);
        if text.0 != rendered {
            text.0 = rendered;
        }
    }
    for (mut node, which) in &mut fills {
        let value = match which.0 {
            Action::GunFire => settings.gun_fire,
            Action::Master => settings.master,
            _ => 0.0,
        };
        let width = Val::Percent(value * 100.0);
        if node.width != width {
            node.width = width;
        }
    }
}

/// Keep Bevy's own master volume in step with the setting, which every cue obeys.
pub(crate) fn apply_volumes(settings: Res<Settings>, mut volume: ResMut<GlobalVolume>) {
    let wanted = Volume::Linear(settings.master.clamp(0.0, 1.0));
    if volume.volume != wanted {
        volume.volume = wanted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every control sits inside the panel and no two overlap: what falls outside could never be
    /// clicked, and an overlap would make one of the two unreachable in that corner.
    #[test]
    fn the_controls_fit_inside_the_panel_without_overlapping() {
        for control in CONTROLS {
            let [x, y, w, h] = control.rect;
            assert!(x >= 0.0 && y >= 0.0, "{} starts outside", control.label);
            assert!(w > 0.0 && h > 0.0, "{} has no size", control.label);
            assert!(
                x + w <= PANEL_SIZE.x && y + h <= PANEL_SIZE.y,
                "{} runs past the panel",
                control.label
            );
            // A slider carries its label and value above its track.
            if control.kind == Kind::Slider {
                assert!(
                    y - 22.0 >= 0.0,
                    "{} has no room for its label",
                    control.label
                );
            }
        }
        for (i, a) in CONTROLS.iter().enumerate() {
            for b in &CONTROLS[i + 1..] {
                let overlaps = a.rect[0] < b.rect[0] + b.rect[2]
                    && b.rect[0] < a.rect[0] + a.rect[2]
                    && a.rect[1] < b.rect[1] + b.rect[3]
                    && b.rect[1] < a.rect[1] + a.rect[3];
                assert!(!overlaps, "{} overlaps {}", a.label, b.label);
            }
        }
    }

    /// Each action has exactly one control, so one cannot be dropped by an edit by accident.
    #[test]
    fn every_action_has_one_control() {
        for action in [
            Action::Grant,
            Action::Prestige,
            Action::ResetPrestige,
            Action::GunFire,
            Action::Master,
            Action::Close,
        ] {
            let count = CONTROLS
                .iter()
                .filter(|control| control.action == action)
                .count();
            assert_eq!(count, 1, "{action:?} has {count} controls");
        }
    }

    /// A point picks the control under it and nowhere else, and a slider's value follows the point
    /// across the track and stops at both ends.
    #[test]
    fn a_point_picks_a_control_and_a_share_of_its_track() {
        let slider = CONTROLS
            .iter()
            .find(|control| control.action == Action::GunFire)
            .expect("a gun-fire slider");
        let [x, y, w, h] = slider.rect;
        assert!(hit(slider.rect, Vec2::new(x + w / 2.0, y + h / 2.0)));
        assert!(!hit(slider.rect, Vec2::new(x - 1.0, y + h / 2.0)));
        assert!(!hit(slider.rect, Vec2::new(x + w / 2.0, y + h + 1.0)));

        assert_eq!(share(slider.rect, Vec2::new(x, y)), 0.0);
        assert_eq!(share(slider.rect, Vec2::new(x + w, y)), 1.0);
        assert!((share(slider.rect, Vec2::new(x + w / 4.0, y)) - 0.25).abs() < f32::EPSILON);
        assert_eq!(
            share(slider.rect, Vec2::new(x - 500.0, y)),
            0.0,
            "dragging off the left end is silence, not a negative level"
        );
        assert_eq!(share(slider.rect, Vec2::new(x + 5000.0, y)), 1.0);

        // The middle of a button is the button, not a slider: the rects do not overlap.
        let button = CONTROLS
            .iter()
            .find(|control| control.action == Action::ResetPrestige)
            .expect("a reset button");
        let centre = Vec2::new(
            button.rect[0] + button.rect[2] / 2.0,
            button.rect[1] + button.rect[3] / 2.0,
        );
        assert!(hit(button.rect, centre));
        assert!(!hit(slider.rect, centre));
    }
}
