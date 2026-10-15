//! The menu: one panel for everything the player sets.
//!
//! It holds two pages behind a tab row. **Character** chooses the character, its body skin, a head
//! (hairstyle) variant and a face (makeup) preset — the four lists the old select screen drew as
//! columns of text, now lists that scroll and cannot run off the screen. **Developer** holds the
//! career controls and the sound levels.
//!
//! It is the game's own Bevy UI, not a second UI stack: `bevy_egui` needs Bevy 0.18 and this tree is
//! on 0.20, and nothing newer is reachable from here, so the panel, its lists, its buttons and its
//! sliders are nodes.
//!
//! The highlight *is* the selection: moving through a list previews the choice on the character, so
//! a hairstyle or a face does not have to be confirmed to be seen. The choice is written to disk
//! when the menu closes, not on every step.
//!
//! Geometry is decided in one place — [`list_rect`], [`row_rect`], [`bar_rect`], [`tab_rect`] and
//! the rest — and both the nodes that are drawn and the pointer that is hit-tested against them
//! read it, so the two cannot drift apart. The panel itself is placed in the middle of the window
//! by [`panel_origin`], which the pointer maths undo.

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::catalog;
use crate::loading::ModelKey;
use crate::progress::{self, Career};
use crate::set_cursor_grab;
use crate::settings::Settings;

/// How many lists the character page has: character, body skin, head, face.
const LISTS: usize = 4;

/// The panel's size in logical pixels. It is centred in the window.
const PANEL_SIZE: Vec2 = Vec2::new(1180.0, 660.0);

/// The padding between the panel's edge and its content.
const PAD: f32 = 20.0;
/// The height of one list row, and how many a list shows at once.
const ROW: f32 = 22.0;
const ROWS: usize = 22;
/// The scrollbar's width and the gap before it.
const BAR: f32 = 12.0;
const BAR_GAP: f32 = 8.0;
/// A list column's width and the gap between columns.
const COL_W: f32 = 270.0;
const COL_GAP: f32 = 20.0;

/// The tab row, and where the page body starts.
const TABS_Y: f32 = 44.0;
const TAB_H: f32 = 30.0;
const TAB_W: f32 = 150.0;
const TAB_GAP: f32 = 8.0;
const BODY_Y: f32 = 92.0;
/// A list's title line and its viewport.
const LIST_TITLE_H: f32 = 24.0;
const LIST_Y: f32 = BODY_Y + LIST_TITLE_H;
const LIST_H: f32 = ROWS as f32 * ROW;

/// The bottom hint line.
const HINT_Y: f32 = PANEL_SIZE.y - 34.0;
/// The "Close" button in the tab row.
const CLOSE_W: f32 = 120.0;

/// The panel-local rect of list `list`'s viewport: `[x, y, width, height]`.
pub(crate) fn list_rect(list: usize) -> [f32; 4] {
    [
        PAD + list as f32 * (COL_W + COL_GAP),
        LIST_Y,
        COL_W - BAR - BAR_GAP,
        LIST_H,
    ]
}

/// The panel-local rect of list `list`'s title line.
fn list_title_rect(list: usize) -> [f32; 4] {
    [
        PAD + list as f32 * (COL_W + COL_GAP),
        BODY_Y,
        COL_W,
        LIST_TITLE_H,
    ]
}

/// The panel-local rect of the `slot`-th visible row of list `list`. Rows tile the viewport exactly.
pub(crate) fn row_rect(list: usize, slot: usize) -> [f32; 4] {
    let [x, y, w, _] = list_rect(list);
    [x, y + slot as f32 * ROW, w, ROW]
}

/// The panel-local rect of list `list`'s scrollbar track.
pub(crate) fn bar_rect(list: usize) -> [f32; 4] {
    [
        PAD + list as f32 * (COL_W + COL_GAP) + COL_W - BAR,
        LIST_Y,
        BAR,
        LIST_H,
    ]
}

/// The panel-local rect of tab `index`.
pub(crate) fn tab_rect(index: usize) -> [f32; 4] {
    [PAD + index as f32 * (TAB_W + TAB_GAP), TABS_Y, TAB_W, TAB_H]
}

/// The panel-local rect of the Close button.
fn close_rect() -> [f32; 4] {
    [PANEL_SIZE.x - PAD - CLOSE_W, TABS_Y, CLOSE_W, TAB_H]
}

/// Where the panel's top-left corner sits in the window: centred, and never off the top-left edge
/// of a window too small to hold it.
pub(crate) fn panel_origin(window: &Window) -> Vec2 {
    Vec2::new(
        ((window.width() - PANEL_SIZE.x) / 2.0).max(PAD),
        ((window.height() - PANEL_SIZE.y) / 2.0).max(PAD),
    )
}

/// Whether `at` is inside a panel-local rect.
fn hit(rect: [f32; 4], at: Vec2) -> bool {
    at.x >= rect[0] && at.x <= rect[0] + rect[2] && at.y >= rect[1] && at.y <= rect[1] + rect[3]
}

/// The share of a track a point sits at: a slider's value, clamped to its ends.
fn share(rect: [f32; 4], at: Vec2) -> f32 {
    ((at.x - rect[0]) / rect[2]).clamp(0.0, 1.0)
}

/// The item a row slot shows, given the list's scroll.
fn item_at(scroll: i32, slot: usize) -> i32 {
    scroll + slot as i32
}

/// The scroll a point down a scrollbar track asks for, for a list of `total` items.
fn scroll_from_bar(total: i32, at: f32) -> i32 {
    let travel = (total - ROWS as i32).max(0);
    let share = (at / LIST_H).clamp(0.0, 1.0);
    (share * travel as f32).round() as i32
}

/// The largest scroll position a list of `total` items allows.
fn max_scroll(total: i32) -> i32 {
    (total - ROWS as i32).max(0)
}

/// Which page the menu is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Page {
    #[default]
    Character,
    Developer,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Character => "CHARACTER",
            Page::Developer => "DEVELOPER",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Page::Character => {
                "up/down step   pgup/pgdn jump   home/end ends   left/right column   enter apply   [ ] page   tab close"
            }
            Page::Developer => {
                "up/down setting   left/right adjust   enter apply   [ ] page   tab close"
            }
        }
    }
}

/// What a button or a slider does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Action {
    Grant,
    Prestige,
    ResetPrestige,
    GunFire,
    Master,
}

/// The developer page's buttons, and the rect each one occupies.
const BUTTONS: &[(Action, [f32; 4], &str)] = &[
    (
        Action::Grant,
        [PAD, 170.0, 320.0, 30.0],
        "Grant 25 000 experience",
    ),
    (Action::Prestige, [PAD, 208.0, 320.0, 30.0], "Prestige now"),
    (
        Action::ResetPrestige,
        [PAD, 246.0, 320.0, 30.0],
        "Reset prestige",
    ),
];

/// The two sliders on the developer page, in focus order: the setting, its label, its track rect.
const SLIDERS: &[(Action, &str, [f32; 4])] = &[
    (
        Action::GunFire,
        "Gun fire volume",
        [PAD, 324.0, 460.0, 16.0],
    ),
    (Action::Master, "Master volume", [PAD, 400.0, 460.0, 16.0]),
];

/// What the developer page's "grant" button awards.
const GRANT: u32 = 25_000;

// ---------------------------------------------------------------------------------------------
// The menu's state

/// The select screen's state: everything the player is choosing, and what is being shown.
#[derive(Resource, Default)]
pub(crate) struct Menu {
    pub(crate) open: bool,
    page: Page,
    /// Every character whose skins the catalog found.
    characters: Vec<catalog::Character>,
    /// Which list the arrows act on.
    column: usize,
    /// The highlighted item in each list: character, body skin, head, face.
    picked: [usize; LISTS],
    /// The first visible row of each list.
    scroll: [i32; LISTS],
    /// Which slider the developer page's arrows act on.
    dev_focus: usize,
    /// Set when the player confirms; the selection is written to disk once, not on every step.
    pub(crate) requested: bool,
}

impl Menu {
    fn character(&self) -> Option<&catalog::Character> {
        self.characters.get(self.picked[0])
    }

    /// The length of each list, in column order.
    pub(crate) fn lengths(&self) -> [usize; LISTS] {
        [
            self.characters.len(),
            self.character().map_or(0, |c| c.skins.len()),
            self.character().map_or(0, |c| c.heads.len()),
            self.character().map_or(0, |c| c.faces.len()),
        ]
    }

    fn count(&self, list: usize) -> usize {
        self.lengths()[list]
    }

    /// The catalogue as the menu needs it: the characters, and the item the player last applied.
    pub(crate) fn fill(&mut self, characters: Vec<catalog::Character>) {
        self.characters = characters;
        self.clamp();
    }

    /// The label of item `index` in list `list`, as a row shows it.
    fn label(&self, list: usize, index: usize) -> String {
        let Some(character) = self.character() else {
            return String::new();
        };
        match list {
            0 => self
                .characters
                .get(index)
                .map(|c| format!("{:>3} {}", c.id, c.name))
                .unwrap_or_default(),
            1 => character
                .skins
                .get(index)
                .map(|s| s.label.clone())
                .unwrap_or_default(),
            2 => character
                .heads
                .get(index)
                .map(|h| h.label.clone())
                .unwrap_or_default(),
            _ => character
                .faces
                .get(index)
                .map(|f| f.label.clone())
                .unwrap_or_default(),
        }
    }

    /// Highlight `index` in `list`, clamped to the list's own length.
    fn pick(&mut self, list: usize, index: usize) {
        let count = self.count(list);
        self.picked[list] = if count == 0 { 0 } else { index.min(count - 1) };
    }

    /// Move the highlight in `list` by one, wrapping.
    fn step(&mut self, list: usize, up: bool) {
        let length = self.count(list);
        if length == 0 {
            return;
        }
        self.picked[list] = if up {
            (self.picked[list] + length - 1) % length
        } else {
            (self.picked[list] + 1) % length
        };
    }

    /// Keep every highlight inside its list (a shorter list after switching character).
    fn clamp(&mut self) {
        for list in 0..LISTS {
            self.pick(list, self.picked[list]);
            self.scroll[list] = self.scroll[list].clamp(0, max_scroll(self.count(list) as i32));
        }
    }

    /// Scroll so the highlighted row of `list` is on screen.
    fn scroll_into_view(&mut self, list: usize) {
        let selected = self.picked[list] as i32;
        if selected < self.scroll[list] {
            self.scroll[list] = selected;
        }
        if selected >= self.scroll[list] + ROWS as i32 {
            self.scroll[list] = selected - ROWS as i32 + 1;
        }
        self.scroll[list] = self.scroll[list].clamp(0, max_scroll(self.count(list) as i32));
    }

    /// Scroll `list` by `delta` rows, within its bounds.
    fn scroll_by(&mut self, list: usize, delta: i32) {
        let max = max_scroll(self.count(list) as i32);
        self.scroll[list] = (self.scroll[list] + delta).clamp(0, max);
    }

    /// Move the highlight in the focused list by a pageful, taking the scroll with it.
    fn page_step(&mut self, up: bool) {
        let list = self.column;
        let delta = ROWS as i32 * if up { -1 } else { 1 };
        self.scroll_by(list, delta);
        let target =
            (self.picked[list] as i32 + delta).clamp(0, self.count(list).saturating_sub(1) as i32);
        self.pick(list, target as usize);
        self.scroll_into_view(list);
    }

    /// The current selection as it is written to disk, and applied from it.
    pub(crate) fn saved(&self) -> crate::settings::Selection {
        let character = self.character();
        let get = |list: usize| -> String {
            let Some(character) = character else {
                return String::new();
            };
            match list {
                1 => character
                    .skins
                    .get(self.picked[1])
                    .map(|s| s.label.clone())
                    .unwrap_or_default(),
                2 => character
                    .heads
                    .get(self.picked[2])
                    .map(|h| h.label.clone())
                    .unwrap_or_default(),
                _ => character
                    .faces
                    .get(self.picked[3])
                    .map(|f| f.label.clone())
                    .unwrap_or_default(),
            }
        };
        crate::settings::Selection {
            character: character.map(|c| c.id.clone()).unwrap_or_default(),
            skin: get(1),
            head: get(2),
            face: get(3),
        }
    }

    /// Move the selection onto `saved`, per column and independently: a character that no longer
    /// exists, or a skin its character no longer ships, leaves that column where it started rather
    /// than failing the whole selection.
    pub(crate) fn restore(&mut self, saved: &crate::settings::Selection) {
        if let Some(character) = self.characters.iter().position(|c| c.id == saved.character) {
            self.picked[0] = character;
        }
        let found = {
            let Some(character) = self.character() else {
                return;
            };
            (
                character.skins.iter().position(|s| s.label == saved.skin),
                character.heads.iter().position(|h| h.label == saved.head),
                character.faces.iter().position(|f| f.label == saved.face),
            )
        };
        if let Some(skin) = found.0 {
            self.picked[1] = skin;
        }
        if let Some(head) = found.1 {
            self.picked[2] = head;
        }
        if let Some(face) = found.2 {
            self.picked[3] = face;
        }
        self.clamp();
        for list in 0..LISTS {
            self.scroll_into_view(list);
        }
    }

    /// The meshes and texture folders the current highlight resolves to.
    pub(crate) fn key(&self) -> Option<ModelKey> {
        self.key_of(self.picked)
    }

    /// The key a given selection resolves to: the body and head meshes and the folders their
    /// textures live in, with the head and face presets overriding the skin's own defaults.
    fn key_of(&self, selection: [usize; LISTS]) -> Option<ModelKey> {
        let character = self.characters.get(selection[0])?;
        let skin = character.skins.get(selection[1])?;
        let head = character.heads.get(selection[2]);
        let face = character.faces.get(selection[3]);
        Some(ModelKey {
            body: skin.body.clone(),
            head: head
                .and_then(|v| v.mesh.clone())
                .unwrap_or_else(|| skin.head.clone()),
            body_dir: skin.material_dir.clone(),
            head_dir: head
                .map(|v| v.material_dir.clone())
                .unwrap_or_else(|| skin.material_dir.clone()),
            face_dir: face.map(|v| v.material_dir.clone()),
        })
    }

    /// The key with one list stepped by `step`, wrapping like the highlight does.
    fn key_stepped(&self, list: usize, step: i32) -> Option<ModelKey> {
        let length = self.count(list) as i32;
        if length == 0 {
            return None;
        }
        let mut selection = self.picked;
        selection[list] = (selection[list] as i32 + step).rem_euclid(length) as usize;
        self.key_of(selection)
    }

    /// The models a step away from the highlight, so stepping either way is instant. Only the
    /// focused list is prefetched: it is the one the next key press will change.
    pub(crate) fn prefetch_keys(&self) -> Vec<ModelKey> {
        let mut keys = Vec::new();
        let current = self.key();
        for step in [-1, 1] {
            if let Some(key) = self.key_stepped(self.column, step)
                && Some(&key) != current.as_ref()
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        keys
    }
}

// ---------------------------------------------------------------------------------------------
// The nodes

/// The panel root, hidden while the menu is closed.
#[derive(Component)]
pub(crate) struct MenuPanel;
/// One page's body; exactly one is visible.
#[derive(Component)]
pub(crate) struct PageBody(Page);
/// A tab's box and its label, so the active one can be picked out.
#[derive(Component)]
pub(crate) struct TabBox(Page);
#[derive(Component)]
pub(crate) struct TabLabel(Page);
/// A list's title line.
#[derive(Component)]
pub(crate) struct ColumnTitle(usize);
/// One visible row of a list.
#[derive(Component)]
pub(crate) struct RowText {
    list: usize,
    slot: usize,
}
/// A list's scrollbar thumb.
#[derive(Component)]
pub(crate) struct ScrollThumb(usize);
/// The line naming the career the developer page is acting on.
#[derive(Component)]
pub(crate) struct DevStatus;
/// The filled part of a slider's track, and the text showing its value.
#[derive(Component)]
pub(crate) struct SliderFill(Action);
#[derive(Component)]
pub(crate) struct SliderValue(Action);
/// The hint line along the bottom.
#[derive(Component)]
pub(crate) struct HintText;
/// A text node the panel never changes after it is built.
#[derive(Component)]
pub(crate) struct FixedText;

/// The colours the menu draws with.
const INK: Color = Color::srgb(0.94, 0.95, 0.98);
const DIM: Color = Color::srgb(0.72, 0.76, 0.84);
const GOLD: Color = Color::srgb(0.98, 0.86, 0.5);
const BOX: Color = Color::srgb(0.16, 0.19, 0.26);
const EDGE: Color = Color::srgba(0.6, 0.68, 0.82, 0.5);

/// Build the panel once, hidden.
pub(crate) fn setup_menu(mut commands: Commands) {
    commands
        .spawn((
            MenuPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(PANEL_SIZE.x),
                height: Val::Px(PANEL_SIZE.y),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.06, 0.09, 0.96)),
            BorderColor::from(Color::srgba(0.65, 0.72, 0.85, 0.55)),
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            text(
                panel,
                [PAD, 10.0, 700.0, 24.0],
                "SPDESCENDANTS",
                16.0,
                GOLD,
                FixedText,
            );
            for (index, page) in [Page::Character, Page::Developer].iter().enumerate() {
                let [x, y, w, h] = tab_rect(index);
                panel
                    .spawn((
                        TabBox(*page),
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
                        BackgroundColor(BOX),
                        BorderColor::from(EDGE),
                    ))
                    .with_children(|tab| {
                        tab.spawn((
                            TabLabel(*page),
                            Text::new(page.name()),
                            TextFont {
                                font_size: bevy::text::FontSize::Px(14.0),
                                ..default()
                            },
                            TextColor(DIM),
                        ));
                    });
            }
            let [x, y, w, h] = close_rect();
            button(panel, [x, y, w, h], "Close (Tab)", FixedText);
            character_page(panel);
            developer_page(panel);
            text(
                panel,
                [PAD, HINT_Y, PANEL_SIZE.x - 2.0 * PAD, 20.0],
                "",
                13.0,
                DIM,
                HintText,
            );
        });
}

/// The character page: four scrolling lists, each with a title and a scrollbar.
fn character_page(panel: &mut ChildSpawnerCommands) {
    panel
        .spawn((
            PageBody(Page::Character),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Inherited,
        ))
        .with_children(|body| {
            for list in 0..LISTS {
                text(
                    body,
                    list_title_rect(list),
                    "",
                    14.0,
                    GOLD,
                    ColumnTitle(list),
                );
                body.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(list_rect(list)[0]),
                        top: Val::Px(list_rect(list)[1]),
                        width: Val::Px(list_rect(list)[2]),
                        height: Val::Px(list_rect(list)[3]),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)),
                ))
                .with_children(|viewport| {
                    for slot in 0..ROWS {
                        let [x, y, w, h] = row_rect(list, slot);
                        let [vx, vy, _, _] = list_rect(list);
                        text(
                            viewport,
                            [x - vx, y - vy, w, h],
                            "",
                            14.0,
                            INK,
                            RowText { list, slot },
                        );
                    }
                });
                let [bx, by, bw, bh] = bar_rect(list);
                body.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(bx),
                        top: Val::Px(by),
                        width: Val::Px(bw),
                        height: Val::Px(bh),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.06)),
                ))
                .with_children(|track| {
                    track.spawn((
                        ScrollThumb(list),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Percent(100.0),
                            height: Val::Px(bh),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.72, 0.76, 0.86)),
                    ));
                });
            }
        });
}

/// The developer page: the career controls and the sound levels.
fn developer_page(panel: &mut ChildSpawnerCommands) {
    panel
        .spawn((
            PageBody(Page::Developer),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|body| {
            text(body, [PAD, BODY_Y, 900.0, 56.0], "", 15.0, INK, DevStatus);
            for (action, rect, label) in BUTTONS {
                button(body, *rect, label, FixedText);
                let _ = action;
            }
            for (action, label, rect) in SLIDERS {
                text(
                    body,
                    [rect[0], rect[1] - 24.0, rect[2] - 90.0, 20.0],
                    label,
                    14.0,
                    INK,
                    FixedText,
                );
                text(
                    body,
                    [rect[0] + rect[2] - 80.0, rect[1] - 24.0, 80.0, 20.0],
                    "",
                    14.0,
                    GOLD,
                    SliderValue(*action),
                );
                body.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(rect[0]),
                        top: Val::Px(rect[1]),
                        width: Val::Px(rect[2]),
                        height: Val::Px(rect[3]),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.16)),
                ))
                .with_children(|track| {
                    track.spawn((
                        SliderFill(*action),
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(GOLD),
                    ));
                });
            }
        });
}

/// A bordered box with a centred label inside it.
fn button<M: Component>(parent: &mut ChildSpawnerCommands, rect: [f32; 4], label: &str, marker: M) {
    let [x, y, w, h] = rect;
    parent
        .spawn((
            marker,
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
            BackgroundColor(BOX),
            BorderColor::from(EDGE),
        ))
        .with_children(|box_| {
            box_.spawn((
                Text::new(label),
                TextFont {
                    font_size: bevy::text::FontSize::Px(14.0),
                    ..default()
                },
                TextColor(INK),
            ));
        });
}

/// A text node at an absolute spot inside the panel, with a marker.
fn text<M: Component>(
    parent: &mut ChildSpawnerCommands,
    rect: [f32; 4],
    body: &str,
    size: f32,
    colour: Color,
    marker: M,
) {
    let [x, y, w, h] = rect;
    parent.spawn((
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

// ---------------------------------------------------------------------------------------------
// Input

/// `Tab` opens and closes; while it is open the arrows walk the lists and the sliders, `[`/`]`
/// change page, and `Enter` applies. Closing applies too: the highlight is the selection, so there
/// is nothing else to confirm.
pub(crate) fn menu_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        menu.open = !menu.open;
        menu.requested = true;
        set_cursor_grab(!menu.open, &mut cursor);
        return;
    }
    if !menu.open {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::Enter) {
        menu.open = false;
        menu.requested = true;
        set_cursor_grab(true, &mut cursor);
        return;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        menu.page = Page::Character;
        return;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        menu.page = Page::Developer;
        return;
    }

    let up = keys.just_pressed(KeyCode::ArrowUp);
    let down = keys.just_pressed(KeyCode::ArrowDown);
    match menu.page {
        Page::Character => {
            if keys.just_pressed(KeyCode::ArrowLeft) {
                menu.column = (menu.column + LISTS - 1) % LISTS;
                let column = menu.column;
                menu.scroll_into_view(column);
            }
            if keys.just_pressed(KeyCode::ArrowRight) {
                menu.column = (menu.column + 1) % LISTS;
                let column = menu.column;
                menu.scroll_into_view(column);
            }
            if up || down {
                let column = menu.column;
                menu.step(column, up);
                menu.clamp();
                menu.scroll_into_view(column);
            }
            if keys.just_pressed(KeyCode::PageUp) {
                menu.page_step(true);
            }
            if keys.just_pressed(KeyCode::PageDown) {
                menu.page_step(false);
            }
            if keys.just_pressed(KeyCode::Home) {
                let column = menu.column;
                menu.pick(column, 0);
                menu.scroll_into_view(column);
            }
            if keys.just_pressed(KeyCode::End) {
                let column = menu.column;
                menu.pick(column, usize::MAX);
                menu.scroll_into_view(column);
            }
        }
        Page::Developer => {
            if up || down {
                let count = SLIDERS.len();
                menu.dev_focus = if up {
                    (menu.dev_focus + count - 1) % count
                } else {
                    (menu.dev_focus + 1) % count
                };
            }
            let adjust = if keys.just_pressed(KeyCode::ArrowLeft) {
                -1
            } else if keys.just_pressed(KeyCode::ArrowRight) {
                1
            } else {
                0
            };
            if adjust != 0 {
                let action = SLIDERS[menu.dev_focus].0;
                let step = adjust as f32 * 0.05;
                if matches!(action, Action::GunFire) {
                    settings.gun_fire = (settings.gun_fire + step).clamp(0.0, 1.0);
                } else {
                    settings.master = (settings.master + step).clamp(0.0, 1.0);
                }
                if let Err(error) = settings.save() {
                    warn!("menu: could not write the settings: {error}");
                }
            }
        }
    }
}

/// A click on a tab, a list row or a control; a drag on a scrollbar or a slider; the wheel over a
/// list scrolls it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn menu_pointer(
    buttons: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut menu: ResMut<Menu>,
    mut settings: ResMut<Settings>,
    mut career: ResMut<Career>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut unsaved: Local<bool>,
) {
    // The wheel is drained whether or not the menu is open, so it cannot pile up behind it.
    let lines: f32 = wheel.read().map(|event| event.y).sum();
    if !menu.open {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(at) = window.cursor_position().map(|at| at - panel_origin(window)) else {
        return;
    };

    let pressed = buttons.pressed(MouseButton::Left);
    let clicked = buttons.just_pressed(MouseButton::Left);
    let released = buttons.just_released(MouseButton::Left);

    if clicked {
        for (index, page) in [Page::Character, Page::Developer].iter().enumerate() {
            if hit(tab_rect(index), at) {
                menu.page = *page;
            }
        }
        if hit(close_rect(), at) {
            menu.open = false;
            menu.requested = true;
            set_cursor_grab(true, &mut cursor);
            return;
        }
    }

    match menu.page {
        Page::Character => {
            for list in 0..LISTS {
                if lines != 0.0 && hit(list_rect(list), at) {
                    menu.scroll_by(list, -lines.round() as i32);
                }
                if clicked && hit(list_rect(list), at) {
                    menu.column = list;
                    let slot = ((at.y - list_rect(list)[1]) / ROW) as usize;
                    let index = item_at(menu.scroll[list], slot);
                    if index >= 0 && (index as usize) < menu.count(list) {
                        menu.pick(list, index as usize);
                    }
                }
                if pressed && hit(bar_rect(list), at) {
                    let total = menu.count(list) as i32;
                    menu.scroll[list] = scroll_from_bar(total, at.y - bar_rect(list)[1]);
                }
            }
        }
        Page::Developer => {
            for (action, rect, _) in BUTTONS {
                if clicked && hit(*rect, at) {
                    match action {
                        Action::Grant => progress::grant(&mut career, GRANT),
                        Action::Prestige => {
                            if !career.prestige() {
                                info!("menu: prestige is not on offer");
                            }
                        }
                        Action::ResetPrestige => {
                            career.reset();
                            info!("menu: prestige reset");
                        }
                        _ => {}
                    }
                }
            }
            for (action, _, rect) in SLIDERS {
                if pressed && hit(*rect, at) {
                    let value = share(*rect, at);
                    match action {
                        Action::GunFire => settings.gun_fire = value,
                        Action::Master => settings.master = value,
                        _ => {}
                    }
                    *unsaved = true;
                }
            }
        }
    }

    // The settings are written when the drag ends, not on every frame of it.
    if *unsaved && (released || !pressed) {
        if let Err(error) = settings.save() {
            warn!("menu: could not write the settings: {error}");
        }
        *unsaved = false;
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing

/// Put the panel in the middle of the window, and show or hide it.
pub(crate) fn place_menu(
    menu: Res<Menu>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut panels: Query<(&mut Node, &mut Visibility), With<MenuPanel>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let origin = panel_origin(window);
    let wanted = if menu.open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (mut node, mut visibility) in &mut panels {
        if node.left != Val::Px(origin.x) {
            node.left = Val::Px(origin.x);
        }
        if node.top != Val::Px(origin.y) {
            node.top = Val::Px(origin.y);
        }
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Show the page that is selected and hide the other.
pub(crate) fn draw_pages(menu: Res<Menu>, mut bodies: Query<(&mut Visibility, &PageBody)>) {
    for (mut visibility, body) in &mut bodies {
        let wanted = if body.0 == menu.page {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
    }
}

/// Pick out the active tab.
pub(crate) fn draw_tabs(
    menu: Res<Menu>,
    mut labels: Query<(&mut TextColor, &TabLabel)>,
    mut boxes: Query<(&mut BackgroundColor, &TabBox)>,
) {
    for (mut colour, label) in &mut labels {
        let wanted = if label.0 == menu.page { GOLD } else { DIM };
        if colour.0 != wanted {
            colour.0 = wanted;
        }
    }
    for (mut colour, tab) in &mut boxes {
        let wanted = if tab.0 == menu.page {
            Color::srgb(0.24, 0.28, 0.38)
        } else {
            BOX
        };
        if colour.0 != wanted {
            colour.0 = wanted;
        }
    }
}

/// Fill the visible rows of each list from the highlight and the scroll, and pick out the
/// highlighted row of the focused list.
pub(crate) fn draw_rows(menu: Res<Menu>, mut titles: TitleQuery, mut rows: RowQuery) {
    for (mut node, title) in &mut titles {
        let list = title.0;
        let marked = if list == menu.column { ">" } else { " " };
        let rendered = format!("{marked} {} ({})", LIST_NAMES[list], menu.count(list));
        if node.0 != rendered {
            node.0 = rendered;
        }
    }
    for (mut node, mut colour, row) in &mut rows {
        let index = item_at(menu.scroll[row.list], row.slot);
        let count = menu.count(row.list) as i32;
        let (rendered, wanted) = if index < 0 || index >= count {
            (String::new(), DIM)
        } else {
            let selected = menu.picked[row.list] == index as usize;
            let marked = if selected && row.list == menu.column {
                ">"
            } else if selected {
                "*"
            } else {
                " "
            };
            (
                format!("{marked} {}", menu.label(row.list, index as usize)),
                if selected { GOLD } else { INK },
            )
        };
        if node.0 != rendered {
            node.0 = rendered;
        }
        if colour.0 != wanted {
            colour.0 = wanted;
        }
    }
}

/// A row's text and colour.
type RowQuery<'w, 's> =
    Query<'w, 's, (&'static mut Text, &'static mut TextColor, &'static RowText)>;
/// A list's title line, told apart from the rows it shares the panel with.
type TitleQuery<'w, 's> =
    Query<'w, 's, (&'static mut Text, &'static ColumnTitle), Without<RowText>>;
/// The career line, the slider values, the slider fills and the hint line all live on the developer
/// page; each is a separate query, so each is told apart from the others by what it marks.
type StatusQuery<'w, 's> =
    Query<'w, 's, &'static mut Text, (With<DevStatus>, Without<SliderValue>, Without<HintText>)>;
type ValueQuery<'w, 's> = Query<'w, 's, (&'static mut Text, &'static SliderValue)>;
type FillQuery<'w, 's> = Query<'w, 's, (&'static mut Node, &'static SliderFill)>;
type HintQuery<'w, 's> =
    Query<'w, 's, &'static mut Text, (With<HintText>, Without<DevStatus>, Without<SliderValue>)>;

/// The four lists' names, in column order.
const LIST_NAMES: [&str; LISTS] = ["CHARACTER", "BODY SKIN", "HEAD", "FACE"];

/// Size each list's scrollbar thumb to the list, and place it at the scroll.
pub(crate) fn draw_scrollbars(menu: Res<Menu>, mut thumbs: Query<(&mut Node, &ScrollThumb)>) {
    for (mut node, thumb) in &mut thumbs {
        let list = thumb.0;
        let total = menu.count(list) as i32;
        let (height, top) = if total <= ROWS as i32 {
            (VAL_ZERO, VAL_ZERO)
        } else {
            let travel = max_scroll(total);
            let thumb_h = (LIST_H * ROWS as f32 / total as f32).max(24.0);
            let offset = (LIST_H - thumb_h) * menu.scroll[list] as f32 / travel as f32;
            (Val::Px(thumb_h), Val::Px(offset))
        };
        if node.height != height {
            node.height = height;
        }
        if node.top != top {
            node.top = top;
        }
    }
}

/// A zero-height thumb, for a list that fits without scrolling.
const VAL_ZERO: Val = Val::Px(0.0);

/// Keep the developer page's readouts current, and show which slider the arrows act on.
pub(crate) fn draw_dev(
    menu: Res<Menu>,
    career: Res<Career>,
    settings: Res<Settings>,
    mut status: StatusQuery,
    mut values: ValueQuery,
    mut fills: FillQuery,
    mut hints: HintQuery,
) {
    for mut hint in &mut hints {
        let rendered = menu.page.hint().to_string();
        if hint.0 != rendered {
            hint.0 = rendered;
        }
    }
    if let Ok(mut text) = status.single_mut() {
        let rendered = if career.is_loaded() {
            let rank = career.rank();
            format!(
                "level {}   rank {}   prestige {}   xp in rank {}\n{}",
                rank.map(|r| r.index + 1).unwrap_or(0),
                rank.map(|r| r.title.clone()).unwrap_or_default(),
                career.progression().prestige(),
                career.progression().xp_in_rank(),
                "the character page's lists scroll; every choice previews on the character",
            )
        } else {
            "no rank table: the career and its HUD stay hidden".to_owned()
        };
        if text.0 != rendered {
            text.0 = rendered;
        }
    }
    for (mut text, which) in &mut values {
        let rendered = format!("{:.0}%", value_of(&settings, which.0) * 100.0);
        if text.0 != rendered {
            text.0 = rendered;
        }
    }
    for (mut node, which) in &mut fills {
        let width = Val::Percent(value_of(&settings, which.0) * 100.0);
        if node.width != width {
            node.width = width;
        }
    }
}

/// The level a slider shows.
fn value_of(settings: &Settings, action: Action) -> f32 {
    match action {
        Action::GunFire => settings.gun_fire,
        Action::Master => settings.master,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Character, Skin, Variant};

    /// The four viewports and their scrollbars sit inside the panel and never overlap: a control
    /// that falls outside could never be clicked, and an overlap would hide one of the two.
    #[test]
    fn the_lists_and_their_scrollbars_fit_inside_the_panel() {
        for list in 0..LISTS {
            for rect in [list_title_rect(list), list_rect(list), bar_rect(list)] {
                assert!(rect[0] >= 0.0 && rect[1] >= 0.0);
                assert!(
                    rect[0] + rect[2] <= PANEL_SIZE.x && rect[1] + rect[3] <= PANEL_SIZE.y,
                    "list {list} runs past the panel"
                );
            }
            assert!(list_rect(list)[1] + list_rect(list)[3] < HINT_Y);
        }
        for a in 0..LISTS {
            for b in a + 1..LISTS {
                assert!(
                    list_rect(a)[0] + list_rect(a)[2] <= list_rect(b)[0],
                    "lists {a} and {b} overlap"
                );
            }
        }
    }

    /// Rows tile their viewport exactly: no gap a click would fall through, and no row past the
    /// bottom edge.
    #[test]
    fn the_rows_tile_their_viewport() {
        for list in 0..LISTS {
            let [_, y, _, h] = list_rect(list);
            let first = row_rect(list, 0);
            let last = row_rect(list, ROWS - 1);
            assert_eq!(first[1], y);
            assert_eq!(last[1] + last[3], y + h);
            for slot in 0..ROWS {
                assert_eq!(row_rect(list, slot)[3], ROW);
                assert_eq!(row_rect(list, slot)[1], y + slot as f32 * ROW);
            }
            // The scrollbar sits beside the rows, not over them.
            assert!(bar_rect(list)[0] >= list_rect(list)[0] + list_rect(list)[2]);
        }
    }

    /// The tabs and the Close button sit in the tab row and never overlap each other.
    #[test]
    fn the_tab_row_holds_the_tabs_and_the_close_button() {
        let mut rects = vec![tab_rect(0), tab_rect(1), close_rect()];
        for rect in &rects {
            assert_eq!(rect[1], TABS_Y);
            assert_eq!(rect[3], TAB_H);
            assert!(
                rect[0] + rect[2] <= PANEL_SIZE.x,
                "a tab row control runs past the panel"
            );
        }
        rects.sort_by(|a, b| a[0].total_cmp(&b[0]));
        for pair in rects.windows(2) {
            assert!(
                pair[0][0] + pair[0][2] <= pair[1][0],
                "tab row controls overlap"
            );
        }
    }

    /// The developer page's controls fit inside the panel's body and do not overlap.
    #[test]
    fn the_developer_controls_fit_and_do_not_overlap() {
        let mut rects: Vec<[f32; 4]> = BUTTONS.iter().map(|(_, rect, _)| *rect).collect();
        rects.extend(SLIDERS.iter().map(|(_, _, rect)| *rect));
        for rect in &rects {
            assert!(
                rect[0] >= 0.0 && rect[1] >= BODY_Y && rect[2] > 0.0 && rect[3] > 0.0,
                "a developer control is outside the body"
            );
            assert!(
                rect[0] + rect[2] <= PANEL_SIZE.x && rect[1] + rect[3] < HINT_Y,
                "a developer control runs past the panel"
            );
        }
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let overlaps = a[0] < b[0] + b[2]
                    && b[0] < a[0] + a[2]
                    && a[1] < b[1] + b[3]
                    && b[1] < a[1] + a[3];
                assert!(!overlaps, "two developer controls overlap");
            }
        }
    }

    /// The pointer arithmetic and the drawn rows agree: the share of a track is its slider's value,
    /// and a point picks the control under it and nowhere else.
    #[test]
    fn the_pointer_agrees_with_what_is_drawn() {
        let track = SLIDERS[0].2;
        let [x, y, w, h] = track;
        assert!(hit(track, Vec2::new(x + w / 2.0, y + h / 2.0)));
        assert!(!hit(track, Vec2::new(x - 1.0, y + h / 2.0)));
        assert_eq!(share(track, Vec2::new(x, y)), 0.0);
        assert_eq!(share(track, Vec2::new(x + w, y)), 1.0);
        assert!((share(track, Vec2::new(x + w / 4.0, y)) - 0.25).abs() < f32::EPSILON);
        assert_eq!(share(track, Vec2::new(x - 500.0, y)), 0.0);
    }

    /// A window smaller than the panel keeps the panel's corner on screen rather than off it, so a
    /// click can still reach the controls.
    #[test]
    fn the_panel_is_centred_and_never_off_screen() {
        let mut window = Window::default();
        window.resolution.set(1280.0, 720.0);
        let origin = panel_origin(&window);
        assert_eq!(
            origin,
            Vec2::new(50.0, 30.0),
            "centred in a 1280x720 window"
        );
        window.resolution.set(640.0, 400.0);
        assert_eq!(panel_origin(&window), Vec2::splat(PAD));
    }

    /// Row slots map onto items through the scroll, and a click down a scrollbar asks for the
    /// position it points at, staying inside the list.
    #[test]
    fn slots_and_scrollbars_map_onto_items() {
        assert_eq!(item_at(0, 0), 0);
        assert_eq!(item_at(5, 3), 8);
        let total = 100;
        assert_eq!(scroll_from_bar(total, 0.0), 0);
        assert_eq!(scroll_from_bar(total, LIST_H), max_scroll(total));
        assert_eq!(
            scroll_from_bar(total, -50.0),
            0,
            "above the track is the top"
        );
        assert_eq!(
            scroll_from_bar(total, LIST_H * 3.0),
            max_scroll(total),
            "below the track is the bottom"
        );
        assert_eq!(max_scroll(10), 0, "a list that fits does not scroll");
        assert_eq!(max_scroll(30), 30 - ROWS as i32);
    }

    /// A menu of `n` items per list, for the navigation tests.
    fn menu_of(n: usize) -> Menu {
        let skins = (0..n)
            .map(|i| Skin {
                label: format!("skin {i}"),
                body: format!("body {i}"),
                head: format!("head {i}"),
                material_dir: format!("dir {i}"),
            })
            .collect();
        let heads = (0..n)
            .map(|i| Variant {
                label: format!("head {i}"),
                mesh: Some(format!("head mesh {i}")),
                material_dir: format!("head dir {i}"),
            })
            .collect();
        let faces = (0..n)
            .map(|i| Variant {
                label: format!("face {i}"),
                mesh: None,
                material_dir: format!("face dir {i}"),
            })
            .collect();
        let mut menu = Menu::default();
        menu.fill(vec![Character {
            id: "004".to_string(),
            name: "Bunny".to_string(),
            skins,
            heads,
            faces,
        }]);
        menu
    }

    /// Stepping past either end of a long list scrolls it, and comes back: the highlight is never
    /// left off screen.
    #[test]
    fn the_scroll_follows_the_highlight() {
        let mut menu = menu_of(60);
        menu.column = 2;
        assert_eq!(menu.scroll[2], 0);
        for _ in 0..ROWS {
            menu.step(2, false);
        }
        menu.scroll_into_view(2);
        assert_eq!(
            menu.scroll[2], 1,
            "one past the last visible row scrolls by one"
        );
        assert!((menu.picked[2] as i32) < menu.scroll[2] + ROWS as i32);
        for _ in 0..ROWS {
            menu.step(2, true);
        }
        menu.scroll_into_view(2);
        assert_eq!(
            menu.scroll[2], 0,
            "walking back up to the top brings the scroll with it"
        );
        // And the scroll can never run past the end of the list.
        for _ in 0..200 {
            menu.scroll_by(2, 1);
        }
        assert_eq!(menu.scroll[2], max_scroll(60));
    }

    /// A pageful moves both the highlight and the scroll, and stops at the ends.
    #[test]
    fn a_page_step_moves_the_highlight_and_the_scroll() {
        let mut menu = menu_of(60);
        menu.column = 1;
        menu.page_step(false);
        assert_eq!(menu.picked[1], ROWS);
        assert_eq!(menu.scroll[1], ROWS as i32);
        for _ in 0..10 {
            menu.page_step(false);
        }
        assert_eq!(menu.picked[1], 59, "the last item, not past it");
        menu.page_step(true);
        assert_eq!(menu.picked[1], 59 - ROWS);
        assert_eq!(
            menu.scroll[1],
            59 - 2 * ROWS as i32 + 1,
            "a page up from the end puts the highlight on the last visible row"
        );
    }

    /// Switching to a shorter list leaves the highlight inside it, and the prefetch follows the
    /// focused list in both directions.
    #[test]
    fn the_highlight_is_kept_inside_its_list_and_the_prefetch_follows_the_focus() {
        let mut menu = menu_of(4);
        menu.column = 1;
        menu.picked[1] = 3;
        // A character whose lists are shorter: the highlight clamps rather than pointing past the
        // end of the list it no longer belongs to.
        let mut shorter = menu_of(2);
        shorter.column = 1;
        shorter.picked = [0, 3, 3, 3];
        shorter.clamp();
        assert_eq!(shorter.picked, [0, 1, 1, 1]);

        menu.picked = [0, 1, 0, 0];
        menu.column = 1;
        let keys = menu.prefetch_keys();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].body, "body 0", "the previous skin");
        assert_eq!(keys[1].body, "body 2", "the next skin");
        assert_eq!(keys[0].head, "head mesh 0", "at the current head");

        menu.column = 3;
        let keys = menu.prefetch_keys();
        assert_eq!(keys[0].face_dir.as_deref(), Some("face dir 3"));
        assert_eq!(keys[1].face_dir.as_deref(), Some("face dir 1"));

        // A one-item list has no neighbours to prefetch, and asking for them must not loop back
        // onto itself.
        let mut single = menu_of(1);
        single.column = 2;
        assert!(single.prefetch_keys().is_empty());
    }

    /// The key the highlight resolves to carries the head and face the lists point at, with the
    /// skin's own head as the fallback.
    #[test]
    fn the_key_resolves_the_head_and_face_over_the_skin() {
        let mut menu = menu_of(3);
        menu.picked = [0, 2, 1, 0];
        let key = menu.key().expect("a key");
        assert_eq!(key.body, "body 2");
        assert_eq!(key.head, "head mesh 1");
        assert_eq!(key.body_dir, "dir 2");
        assert_eq!(key.head_dir, "head dir 1");
        assert_eq!(key.face_dir.as_deref(), Some("face dir 0"));
    }

    /// The selection written to disk is the labels the menu shows, and restoring reads them back.
    #[test]
    fn the_selection_round_trips_through_its_saved_form() {
        let mut menu = menu_of(3);
        menu.picked = [0, 2, 1, 0];
        let saved = menu.saved();
        assert_eq!(saved.character, "004");
        assert_eq!(saved.skin, "skin 2");
        assert_eq!(saved.head, "head 1");
        assert_eq!(saved.face, "face 0");
        let mut restored = menu_of(3);
        restored.restore(&saved);
        assert_eq!(restored.picked, [0, 2, 1, 0]);
    }

    /// Restoring a selection the catalog no longer holds leaves that column alone instead of
    /// failing the whole thing.
    #[test]
    fn restoring_ignores_labels_that_no_longer_exist() {
        let mut menu = menu_of(3);
        menu.restore(&crate::settings::Selection {
            character: "not here".to_string(),
            skin: "skin 2".to_string(),
            head: "gone".to_string(),
            face: "face 1".to_string(),
        });
        assert_eq!(menu.picked, [0, 2, 0, 1]);
    }
}
