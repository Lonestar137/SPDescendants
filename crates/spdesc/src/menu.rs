//! The menu: one panel for everything the player sets.
//!
//! It holds three pages behind a tab row. **Map** chooses the ground the game is played on — the
//! project's own arena, or a map read out of the player's Modern Warfare 2 install, with that map's
//! own preview art beside the list. **Character** chooses the character, its body skin, a head
//! (hairstyle) variant and a face (makeup) preset. **Developer** holds the career controls and the
//! sound levels.
//!
//! It is the game's own Bevy UI, not a second UI stack: `bevy_egui` needs Bevy 0.18 and this tree is
//! on 0.20, and nothing newer is reachable from here, so the panel, its lists, its buttons and its
//! sliders are nodes.
//!
//! The highlight *is* the selection: moving through a list previews the choice, so a hairstyle, a
//! face or a map does not have to be confirmed to be seen. The choice is written to disk when the
//! menu closes, not on every step.
//!
//! Geometry is decided in one place — [`Pane::rect`] and the rest — and both the nodes that are
//! drawn and the pointer that is hit-tested against them read it, so the two cannot drift apart.
//! The panel itself is placed in the middle of the window by [`panel_origin`], which the pointer
//! maths undo.

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::ui::widget::ImageNode;
use bevy::window::{CursorOptions, PrimaryWindow};

use spdesc_game_core as game_core;

use crate::catalog;
use crate::loading::ModelKey;
use crate::map::{MapRow, Previews};
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
/// A list column's width and the gap between columns, and the map list's own width.
const COL_W: f32 = 270.0;
const COL_GAP: f32 = 20.0;
const MAP_W: f32 = 400.0;

/// The map preview: a 16:9 frame of the highlighted map's own art, beside the list.
const PREVIEW_X: f32 = PAD + MAP_W + 24.0;
const PREVIEW_Y: f32 = BODY_Y + 34.0;
const PREVIEW_W: f32 = PANEL_SIZE.x - PREVIEW_X - PAD;
const PREVIEW_H: f32 = PREVIEW_W * 9.0 / 16.0;

/// The preview's frame, panel-local: `[x, y, width, height]`.
fn preview_rect() -> [f32; 4] {
    [PREVIEW_X, PREVIEW_Y, PREVIEW_W, PREVIEW_H]
}

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

/// One scrollable list of choices. The character page has four side by side, the map page has one,
/// and the developer page has none.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Pane {
    /// A list on the character page, by column.
    Character(usize),
    /// The map list, which is also what the menu opens on.
    #[default]
    Map,
}

/// The lists on the character page, in the order the arrows and the pointer walk them.
const CHARACTER_PANES: [Pane; LISTS] = [
    Pane::Character(0),
    Pane::Character(1),
    Pane::Character(2),
    Pane::Character(3),
];
/// The lists on the map page.
const MAP_PANES: [Pane; 1] = [Pane::Map];

/// The lists `page` shows.
fn panes(page: Page) -> &'static [Pane] {
    match page {
        Page::Map => &MAP_PANES,
        Page::Character => &CHARACTER_PANES,
        Page::Developer => &[],
    }
}

impl Pane {
    /// Where the list starts and how wide it is. The map list is one wide column; the character
    /// page's four share the panel between them.
    fn column(self) -> (f32, f32) {
        match self {
            Pane::Character(list) => (PAD + list as f32 * (COL_W + COL_GAP), COL_W),
            Pane::Map => (PAD, MAP_W),
        }
    }

    /// Its viewport: `[x, y, width, height]`, panel-local.
    pub(crate) fn rect(self) -> [f32; 4] {
        let (x, width) = self.column();
        [x, LIST_Y, width - BAR - BAR_GAP, LIST_H]
    }

    /// Its title line.
    fn title_rect(self) -> [f32; 4] {
        let (x, width) = self.column();
        [x, BODY_Y, width, LIST_TITLE_H]
    }

    /// Its scrollbar track.
    pub(crate) fn bar_rect(self) -> [f32; 4] {
        let (x, width) = self.column();
        [x + width - BAR, LIST_Y, BAR, LIST_H]
    }

    /// The `slot`-th visible row, relative to the list's own viewport. Rows tile it exactly.
    fn row_rect(self, slot: usize) -> [f32; 4] {
        let [_, _, w, _] = self.rect();
        [0.0, slot as f32 * ROW, w, ROW]
    }

    /// What the list is called, with its count appended where it is drawn.
    fn name(self) -> &'static str {
        match self {
            Pane::Character(0) => "CHARACTER",
            Pane::Character(1) => "BODY SKIN",
            Pane::Character(2) => "HEAD",
            Pane::Character(_) => "FACE",
            Pane::Map => "MAP",
        }
    }
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

/// Which page the menu is showing. It opens on the map, which is the first choice a game asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Page {
    #[default]
    Map,
    Character,
    Developer,
}

/// The pages, in tab order.
const PAGES: [Page; 3] = [Page::Map, Page::Character, Page::Developer];

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Map => "MAP",
            Page::Character => "CHARACTER",
            Page::Developer => "DEVELOPER",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Page::Map => {
                "up/down step   pgup/pgdn jump   home/end ends   enter apply   [ ] page   tab close"
            }
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

/// Everything the player is choosing, and what is being shown.
#[derive(Resource, Default)]
pub(crate) struct Menu {
    pub(crate) open: bool,
    page: Page,
    /// Every character whose skins the catalog found.
    characters: Vec<catalog::Character>,
    /// Every map on offer, the project's own first, and the rule the choices resolve against.
    map_rows: Vec<MapRow>,
    maps: game_core::Maps,
    /// Which list the arrows act on.
    focus: Pane,
    /// The highlighted item in each character list: character, body skin, head, face.
    picked: [usize; LISTS],
    /// The first visible row of each character list.
    scroll: [i32; LISTS],
    /// The highlighted map, and the first visible row of its list.
    map_pick: usize,
    map_scroll: i32,
    /// Which slider the developer page's arrows act on.
    dev_focus: usize,
    /// Set when the player confirms; the selection is written to disk once, not on every step.
    pub(crate) requested: bool,
}

impl Menu {
    fn character(&self) -> Option<&catalog::Character> {
        self.characters.get(self.picked[0])
    }

    /// The length of each character list, in column order.
    pub(crate) fn lengths(&self) -> [usize; LISTS] {
        [
            self.characters.len(),
            self.character().map_or(0, |c| c.skins.len()),
            self.character().map_or(0, |c| c.heads.len()),
            self.character().map_or(0, |c| c.faces.len()),
        ]
    }

    /// How many items a list holds.
    fn count(&self, pane: Pane) -> usize {
        match pane {
            Pane::Character(list) => self.lengths()[list],
            Pane::Map => self.map_rows.len(),
        }
    }

    /// The highlight in a list.
    fn selected(&self, pane: Pane) -> usize {
        match pane {
            Pane::Character(list) => self.picked[list],
            Pane::Map => self.map_pick,
        }
    }

    /// The first visible row of a list.
    fn scroll_of(&self, pane: Pane) -> i32 {
        match pane {
            Pane::Character(list) => self.scroll[list],
            Pane::Map => self.map_scroll,
        }
    }

    fn set_scroll(&mut self, pane: Pane, value: i32) {
        match pane {
            Pane::Character(list) => self.scroll[list] = value,
            Pane::Map => self.map_scroll = value,
        }
    }

    /// The characters the catalog found, and the selection the player last applied.
    pub(crate) fn fill(&mut self, characters: Vec<catalog::Character>) {
        self.characters = characters;
        self.clamp();
    }

    /// The maps on offer, and the id the player last chose.
    pub(crate) fn fill_maps(&mut self, rows: Vec<MapRow>, saved: &str) {
        self.maps = match rows.split_first() {
            Some((first, rest)) => {
                game_core::Maps::new(first.map.clone(), rest.iter().map(|row| row.map.clone()))
            }
            None => game_core::Maps::default(),
        };
        self.map_rows = rows;
        // The catalogue owns the rule: an id this install no longer ships lands on the default.
        self.map_pick = self
            .maps
            .resolve(saved)
            .and_then(|map| self.map_rows.iter().position(|row| row.map.id == map.id))
            .unwrap_or(0);
        self.clamp();
        self.scroll_into_view(Pane::Map);
    }

    /// The map being played: its title, and whether the practice targets stand on it.
    pub(crate) fn chosen_map(&self) -> (String, bool) {
        match self.map_rows.get(self.map_pick) {
            Some(row) => (row.map.title.clone(), row.map.targets),
            None => (String::new(), false),
        }
    }

    /// The highlighted map, for the art and detail the menu shows beside the list.
    pub(crate) fn map_row(&self) -> Option<&MapRow> {
        self.map_rows.get(self.map_pick)
    }

    /// The label of item `index` in a list, as a row shows it.
    fn label(&self, pane: Pane, index: usize) -> String {
        if let Pane::Map = pane {
            return self
                .map_rows
                .get(index)
                .map(|row| row.map.title.clone())
                .unwrap_or_default();
        }
        let Pane::Character(list) = pane else {
            return String::new();
        };
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

    /// Highlight `index` in a list, clamped to the list's own length.
    fn pick(&mut self, pane: Pane, index: usize) {
        let count = self.count(pane);
        let index = if count == 0 { 0 } else { index.min(count - 1) };
        match pane {
            Pane::Character(list) => self.picked[list] = index,
            Pane::Map => self.map_pick = index,
        }
    }

    /// Move the highlight in a list by one, wrapping.
    fn step(&mut self, pane: Pane, up: bool) {
        let length = self.count(pane);
        if length == 0 {
            return;
        }
        let index = self.selected(pane);
        let next = if up {
            (index + length - 1) % length
        } else {
            (index + 1) % length
        };
        self.pick(pane, next);
    }

    /// Keep every highlight inside its list (shorter lists after a change, or a new catalogue).
    fn clamp(&mut self) {
        for pane in CHARACTER_PANES {
            self.pick(pane, self.selected(pane));
            let max = max_scroll(self.count(pane) as i32);
            self.set_scroll(pane, self.scroll_of(pane).clamp(0, max));
        }
        self.pick(Pane::Map, self.map_pick);
        let max = max_scroll(self.count(Pane::Map) as i32);
        self.set_scroll(Pane::Map, self.map_scroll.clamp(0, max));
    }

    /// Scroll so a list's highlighted row is on screen.
    fn scroll_into_view(&mut self, pane: Pane) {
        let selected = self.selected(pane) as i32;
        let mut scroll = self.scroll_of(pane);
        if selected < scroll {
            scroll = selected;
        }
        if selected >= scroll + ROWS as i32 {
            scroll = selected - ROWS as i32 + 1;
        }
        let max = max_scroll(self.count(pane) as i32);
        self.set_scroll(pane, scroll.clamp(0, max));
    }

    /// Scroll a list by `delta` rows, within its bounds.
    fn scroll_by(&mut self, pane: Pane, delta: i32) {
        let max = max_scroll(self.count(pane) as i32);
        self.set_scroll(pane, (self.scroll_of(pane) + delta).clamp(0, max));
    }

    /// Move the highlight in the focused list by a pageful, taking the scroll with it.
    fn page_step(&mut self, up: bool) {
        let pane = self.focus;
        let delta = ROWS as i32 * if up { -1 } else { 1 };
        self.scroll_by(pane, delta);
        let target = (self.selected(pane) as i32 + delta)
            .clamp(0, self.count(pane).saturating_sub(1) as i32);
        self.pick(pane, target as usize);
        self.scroll_into_view(pane);
    }

    /// Walk the focused list: a step, a page, or an end.
    fn walk(&mut self, up: bool, down: bool, page: Option<bool>, end: Option<bool>) {
        let pane = self.focus;
        if up || down {
            self.step(pane, up);
            self.clamp();
            self.scroll_into_view(pane);
        }
        if let Some(page_up) = page {
            self.page_step(page_up);
        }
        if let Some(last) = end {
            self.pick(pane, if last { usize::MAX } else { 0 });
            self.scroll_into_view(pane);
        }
    }

    /// Put the focus on a list the page actually shows, and take the page's list with it.
    fn focus_page(&mut self) {
        let lists = panes(self.page);
        if !lists.contains(&self.focus)
            && let Some(first) = lists.first()
        {
            self.focus = *first;
        }
        self.scroll_into_view(self.focus);
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

    /// The map the player chose, as it is written to disk.
    pub(crate) fn saved_map(&self) -> String {
        self.map_rows
            .get(self.map_pick)
            .map(|row| row.map.id.clone())
            .unwrap_or_default()
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
        for pane in CHARACTER_PANES {
            self.scroll_into_view(pane);
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

    /// The key with one character list stepped by `step`, wrapping like the highlight does.
    fn key_stepped(&self, list: usize, step: i32) -> Option<ModelKey> {
        let length = self.lengths()[list] as i32;
        if length == 0 {
            return None;
        }
        let mut selection = self.picked;
        selection[list] = (selection[list] as i32 + step).rem_euclid(length) as usize;
        self.key_of(selection)
    }

    /// The models a step away from the highlight, so stepping either way is instant. Only the
    /// focused character list is prefetched: it is the one the next key press will change, and a
    /// map has no model to prefetch.
    pub(crate) fn prefetch_keys(&self) -> Vec<ModelKey> {
        let Pane::Character(list) = self.focus else {
            return Vec::new();
        };
        let mut keys = Vec::new();
        let current = self.key();
        for step in [-1, 1] {
            if let Some(key) = self.key_stepped(list, step)
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
pub(crate) struct ColumnTitle(Pane);
/// One visible row of a list.
#[derive(Component)]
pub(crate) struct RowText {
    pane: Pane,
    slot: usize,
}
/// A list's scrollbar thumb.
#[derive(Component)]
pub(crate) struct ScrollThumb(Pane);
/// The map page's preview art, filled in once the install's art has been read.
#[derive(Component)]
pub(crate) struct MapPreview;
/// The map page's line of detail about the highlighted map.
#[derive(Component)]
pub(crate) struct MapDetail;
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
            for (index, page) in PAGES.iter().enumerate() {
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
            map_page(panel);
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

/// The map page: the list, and the highlighted map's own art and detail beside it.
fn map_page(panel: &mut ChildSpawnerCommands) {
    panel
        .spawn((
            PageBody(Page::Map),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Inherited,
        ))
        .with_children(|body| {
            list(body, Pane::Map);
            let [px, py, pw, ph] = preview_rect();
            body.spawn((
                MapPreview,
                ImageNode::default(),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(px),
                    top: Val::Px(py),
                    width: Val::Px(pw),
                    height: Val::Px(ph),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.05)),
                BorderColor::from(EDGE),
            ));
            text(
                body,
                [px, py + ph + 16.0, pw, 48.0],
                "",
                13.0,
                DIM,
                MapDetail,
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
            Visibility::Hidden,
        ))
        .with_children(|body| {
            for pane in CHARACTER_PANES {
                list(body, pane);
            }
        });
}

/// One list: its title, its rows, and its scrollbar.
fn list(parent: &mut ChildSpawnerCommands, pane: Pane) {
    text(parent, pane.title_rect(), "", 14.0, GOLD, ColumnTitle(pane));
    let [x, y, w, h] = pane.rect();
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x),
                top: Val::Px(y),
                width: Val::Px(w),
                height: Val::Px(h),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)),
        ))
        .with_children(|viewport| {
            for slot in 0..ROWS {
                text(
                    viewport,
                    pane.row_rect(slot),
                    "",
                    14.0,
                    INK,
                    RowText { pane, slot },
                );
            }
        });
    let [bx, by, bw, bh] = pane.bar_rect();
    parent
        .spawn((
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
                ScrollThumb(pane),
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
            for (_, rect, label) in BUTTONS {
                button(body, *rect, label, FixedText);
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
    if keys.just_pressed(KeyCode::BracketLeft) || keys.just_pressed(KeyCode::BracketRight) {
        let back = keys.just_pressed(KeyCode::BracketLeft);
        let at = PAGES
            .iter()
            .position(|page| *page == menu.page)
            .unwrap_or(0);
        menu.page = PAGES[if back {
            (at + PAGES.len() - 1) % PAGES.len()
        } else {
            (at + 1) % PAGES.len()
        }];
        menu.focus_page();
        return;
    }

    let up = keys.just_pressed(KeyCode::ArrowUp);
    let down = keys.just_pressed(KeyCode::ArrowDown);
    let page = if keys.just_pressed(KeyCode::PageUp) {
        Some(true)
    } else if keys.just_pressed(KeyCode::PageDown) {
        Some(false)
    } else {
        None
    };
    let end = if keys.just_pressed(KeyCode::Home) {
        Some(false)
    } else if keys.just_pressed(KeyCode::End) {
        Some(true)
    } else {
        None
    };
    match menu.page {
        Page::Map => menu.walk(up, down, page, end),
        Page::Character => {
            if keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::ArrowRight) {
                let back = keys.just_pressed(KeyCode::ArrowLeft);
                let at = CHARACTER_PANES
                    .iter()
                    .position(|pane| *pane == menu.focus)
                    .unwrap_or(0);
                menu.focus = CHARACTER_PANES[if back {
                    (at + LISTS - 1) % LISTS
                } else {
                    (at + 1) % LISTS
                }];
                let pane = menu.focus;
                menu.scroll_into_view(pane);
            }
            menu.walk(up, down, page, end);
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
        for (index, page) in PAGES.iter().enumerate() {
            if hit(tab_rect(index), at) {
                menu.page = *page;
                menu.focus_page();
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
        Page::Map | Page::Character => {
            for pane in panes(menu.page) {
                let rect = pane.rect();
                if lines != 0.0 && hit(rect, at) {
                    menu.scroll_by(*pane, -lines.round() as i32);
                }
                if clicked && hit(rect, at) {
                    menu.focus = *pane;
                    let slot = ((at.y - rect[1]) / ROW) as usize;
                    let index = item_at(menu.scroll_of(*pane), slot);
                    if index >= 0 && (index as usize) < menu.count(*pane) {
                        menu.pick(*pane, index as usize);
                    }
                }
                if pressed && hit(pane.bar_rect(), at) {
                    let total = menu.count(*pane) as i32;
                    let scroll = scroll_from_bar(total, at.y - pane.bar_rect()[1]);
                    menu.set_scroll(*pane, scroll);
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

/// Show the page that is selected and hide the others.
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

/// Fill the visible rows of every list from its highlight and scroll, and pick out the highlighted
/// row of the focused list.
pub(crate) fn draw_rows(menu: Res<Menu>, mut titles: TitleQuery, mut rows: RowQuery) {
    for (mut node, title) in &mut titles {
        let pane = title.0;
        let marked = if pane == menu.focus { ">" } else { " " };
        let rendered = format!("{marked} {} ({})", pane.name(), menu.count(pane));
        if node.0 != rendered {
            node.0 = rendered;
        }
    }
    for (mut node, mut colour, row) in &mut rows {
        let pane = row.pane;
        let index = item_at(menu.scroll_of(pane), row.slot);
        let count = menu.count(pane) as i32;
        let (rendered, wanted) = if index < 0 || index >= count {
            (String::new(), DIM)
        } else {
            let selected = menu.selected(pane) == index as usize;
            let marked = if selected && pane == menu.focus {
                ">"
            } else if selected {
                "*"
            } else {
                " "
            };
            (
                format!("{marked} {}", menu.label(pane, index as usize)),
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
type StatusQuery<'w, 's> = Query<
    'w,
    's,
    &'static mut Text,
    (
        With<DevStatus>,
        Without<SliderValue>,
        Without<HintText>,
        Without<MapDetail>,
    ),
>;
type ValueQuery<'w, 's> = Query<'w, 's, (&'static mut Text, &'static SliderValue)>;
type FillQuery<'w, 's> = Query<'w, 's, (&'static mut Node, &'static SliderFill)>;
type HintQuery<'w, 's> = Query<
    'w,
    's,
    &'static mut Text,
    (
        With<HintText>,
        Without<DevStatus>,
        Without<SliderValue>,
        Without<MapDetail>,
    ),
>;
/// The map page's detail line, told apart from the hint and the career line it shares text with.
type DetailQuery<'w, 's> = Query<
    'w,
    's,
    &'static mut Text,
    (
        With<MapDetail>,
        Without<DevStatus>,
        Without<SliderValue>,
        Without<HintText>,
    ),
>;

/// Size each list's scrollbar thumb to the list, and place it at the scroll.
pub(crate) fn draw_scrollbars(menu: Res<Menu>, mut thumbs: Query<(&mut Node, &ScrollThumb)>) {
    for (mut node, thumb) in &mut thumbs {
        let pane = thumb.0;
        let total = menu.count(pane) as i32;
        let (height, top) = if total <= ROWS as i32 {
            (VAL_ZERO, VAL_ZERO)
        } else {
            let travel = max_scroll(total);
            let thumb_h = (LIST_H * ROWS as f32 / total as f32).max(24.0);
            let offset = (LIST_H - thumb_h) * menu.scroll_of(pane) as f32 / travel as f32;
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

/// Show the highlighted map's own art and what the install says about it.
pub(crate) fn draw_map(
    menu: Res<Menu>,
    previews: Res<Previews>,
    mut frames: Query<&mut ImageNode, With<MapPreview>>,
    mut details: DetailQuery,
) {
    let Some(row) = menu.map_row() else {
        return;
    };
    let wanted = previews.handle(&row.map.id);
    for mut frame in &mut frames {
        let current = frame.image.id();
        let next = wanted.as_ref().map(Handle::id).unwrap_or_default();
        if current != next {
            frame.image = wanted.clone().unwrap_or_default();
        }
    }
    let targets = if row.map.targets {
        "practice targets stand here"
    } else {
        "no practice targets — they stand on the dev arena"
    };
    let rendered = format!("{}\n{targets}", row.detail);
    for mut detail in &mut details {
        if detail.0 != rendered {
            detail.0 = rendered.clone();
        }
    }
}

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
                "the character page's lists scroll; every choice previews as it is highlighted",
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
    use spdesc_game_core::{DEV_MAP, GameMap};

    /// Every list of every page and its scrollbar sit inside the panel and never overlap: a control
    /// that falls outside could never be clicked, and an overlap would hide one of the two.
    #[test]
    fn the_lists_and_their_scrollbars_fit_inside_the_panel() {
        for pane in PAGES.iter().flat_map(|page| panes(*page)) {
            for rect in [pane.title_rect(), pane.rect(), pane.bar_rect()] {
                assert!(rect[0] >= 0.0 && rect[1] >= 0.0);
                assert!(
                    rect[0] + rect[2] <= PANEL_SIZE.x && rect[1] + rect[3] <= PANEL_SIZE.y,
                    "{pane:?} runs past the panel"
                );
            }
            assert!(pane.rect()[1] + pane.rect()[3] < HINT_Y);
        }
        for a in CHARACTER_PANES {
            for b in CHARACTER_PANES {
                if a == b {
                    continue;
                }
                assert!(
                    a.rect()[0] + a.rect()[2] <= b.rect()[0]
                        || b.rect()[0] + b.rect()[2] <= a.rect()[0],
                    "{a:?} and {b:?} overlap"
                );
            }
        }
    }

    /// Rows tile their viewport exactly: no gap a click would fall through, and no row past the
    /// bottom edge.
    #[test]
    fn the_rows_tile_their_viewport() {
        for pane in PAGES.iter().flat_map(|page| panes(*page)) {
            let [_, _, _, h] = pane.rect();
            let first = pane.row_rect(0);
            let last = pane.row_rect(ROWS - 1);
            assert_eq!(
                first[1], 0.0,
                "rows are placed from the viewport's own corner"
            );
            assert_eq!(last[1] + last[3], h);
            for slot in 0..ROWS {
                assert_eq!(pane.row_rect(slot)[3], ROW);
                assert_eq!(pane.row_rect(slot)[1], slot as f32 * ROW);
            }
            // The scrollbar sits beside the rows, not over them.
            assert!(pane.bar_rect()[0] >= pane.rect()[0] + pane.rect()[2]);
        }
    }

    /// The map page's preview sits inside the panel, clear of the list it belongs to.
    #[test]
    fn the_map_preview_fits_beside_its_list() {
        let list = Pane::Map.rect();
        let [px, py, pw, ph] = preview_rect();
        assert!(px >= list[0] + list[2], "the preview overlaps the map list");
        assert!(py >= BODY_Y);
        assert!(px + pw <= PANEL_SIZE.x - PAD + 0.001);
        assert!(py + ph + 48.0 <= HINT_Y);
        assert!(
            (pw / ph - 16.0 / 9.0).abs() < 0.001,
            "the preview is not a 16:9 frame"
        );
    }

    /// The tab row holds a tab for every page and the Close button, and they never overlap.
    #[test]
    fn the_tab_row_holds_the_tabs_and_the_close_button() {
        let mut rects: Vec<[f32; 4]> = (0..PAGES.len()).map(tab_rect).collect();
        rects.push(close_rect());
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

    /// A menu of `n` items per character list, for the navigation tests.
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

    /// A row as the map module builds one, without needing an install.
    fn map_row(id: &str, title: &str, targets: bool) -> MapRow {
        MapRow {
            map: GameMap {
                id: id.to_owned(),
                title: title.to_owned(),
                targets,
            },
            preview: None,
            detail: String::new(),
        }
    }

    /// The menu opens on the map page, and the map page is the first tab.
    #[test]
    fn the_menu_opens_on_the_map() {
        let menu = Menu::default();
        assert_eq!(menu.page, Page::Map);
        assert_eq!(menu.focus, Pane::Map);
        assert_eq!(PAGES[0], Page::Map);
        assert_eq!(PAGES.len(), 3);
    }

    /// A map choice is saved by id and comes back, and an id the catalogue does not hold lands on
    /// the project's own map rather than on nothing.
    #[test]
    fn a_map_choice_round_trips_and_an_unknown_id_lands_on_the_default() {
        let rows = vec![
            map_row(DEV_MAP, "Dev arena", true),
            map_row("mp_rust", "Rust", false),
            map_row("mp_terminal", "Terminal", false),
        ];
        let mut menu = Menu::default();
        menu.fill_maps(rows, "mp_terminal");
        assert_eq!(menu.map_pick, 2);
        assert_eq!(menu.saved_map(), "mp_terminal");
        assert_eq!(menu.chosen_map(), ("Terminal".to_owned(), false));
        assert_eq!(menu.count(Pane::Map), 3);
        assert_eq!(menu.label(Pane::Map, 1), "Rust");

        // The project's own map is what a fresh game and an unknown choice both land on.
        let mut fresh = Menu::default();
        fresh.fill_maps(
            vec![
                map_row(DEV_MAP, "Dev arena", true),
                map_row("mp_rust", "Rust", false),
            ],
            "",
        );
        assert_eq!(fresh.map_pick, 0);
        assert_eq!(fresh.chosen_map(), ("Dev arena".to_owned(), true));
        let mut unknown = Menu::default();
        unknown.fill_maps(
            vec![
                map_row(DEV_MAP, "Dev arena", true),
                map_row("mp_rust", "Rust", false),
            ],
            "mp_vacant",
        );
        assert_eq!(
            unknown.map_pick, 0,
            "a map this install lacks is the default"
        );
        assert!(unknown.chosen_map().1, "which is where the targets stand");
    }

    /// Stepping past either end of a long list scrolls it, and comes back: the highlight is never
    /// left off screen.
    #[test]
    fn the_scroll_follows_the_highlight() {
        let mut menu = menu_of(60);
        menu.focus = Pane::Character(2);
        assert_eq!(menu.scroll_of(Pane::Character(2)), 0);
        for _ in 0..ROWS {
            menu.step(Pane::Character(2), false);
        }
        menu.scroll_into_view(Pane::Character(2));
        let head = Pane::Character(2);
        assert_eq!(
            menu.scroll_of(head),
            1,
            "one past the last visible row scrolls by one"
        );
        assert!((menu.selected(head) as i32) < menu.scroll_of(head) + ROWS as i32);
        for _ in 0..ROWS {
            menu.step(head, true);
        }
        menu.scroll_into_view(head);
        assert_eq!(
            menu.scroll_of(head),
            0,
            "walking back up to the top brings the scroll with it"
        );
        // And the scroll can never run past the end of the list.
        for _ in 0..200 {
            menu.scroll_by(head, 1);
        }
        assert_eq!(menu.scroll_of(head), max_scroll(60));
    }

    /// A pageful moves both the highlight and the scroll, and stops at the ends.
    #[test]
    fn a_page_step_moves_the_highlight_and_the_scroll() {
        let mut menu = menu_of(60);
        let skin = Pane::Character(1);
        menu.focus = skin;
        menu.page_step(false);
        assert_eq!(menu.selected(skin), ROWS);
        assert_eq!(menu.scroll_of(skin), ROWS as i32);
        for _ in 0..10 {
            menu.page_step(false);
        }
        assert_eq!(menu.selected(skin), 59, "the last item, not past it");
        menu.page_step(true);
        assert_eq!(menu.selected(skin), 59 - ROWS);
        assert_eq!(
            menu.scroll_of(skin),
            59 - 2 * ROWS as i32 + 1,
            "a page up from the end puts the highlight on the last visible row"
        );
    }

    /// Switching to a shorter list leaves the highlight inside it, and the prefetch follows the
    /// focused character list in both directions — and does nothing on the map page.
    #[test]
    fn the_highlight_is_kept_inside_its_list_and_the_prefetch_follows_the_focus() {
        let mut shorter = menu_of(2);
        shorter.focus = Pane::Character(1);
        shorter.picked = [0, 3, 3, 3];
        shorter.clamp();
        assert_eq!(shorter.picked, [0, 1, 1, 1]);

        let mut menu = menu_of(4);
        menu.picked = [0, 1, 0, 0];
        menu.focus = Pane::Character(1);
        let keys = menu.prefetch_keys();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].body, "body 0", "the previous skin");
        assert_eq!(keys[1].body, "body 2", "the next skin");
        assert_eq!(keys[0].head, "head mesh 0", "at the current head");

        menu.focus = Pane::Character(3);
        let keys = menu.prefetch_keys();
        assert_eq!(keys[0].face_dir.as_deref(), Some("face dir 3"));
        assert_eq!(keys[1].face_dir.as_deref(), Some("face dir 1"));

        // A one-item list has no neighbours to prefetch, and a map has no model at all.
        let mut single = menu_of(1);
        single.focus = Pane::Character(2);
        assert!(single.prefetch_keys().is_empty());
        menu.focus = Pane::Map;
        assert!(menu.prefetch_keys().is_empty());
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

    /// Changing page leaves the focus on a list that page actually shows, so the arrows are never
    /// walking something invisible.
    #[test]
    fn the_focus_stays_on_a_list_the_page_shows() {
        let mut menu = Menu {
            focus: Pane::Character(2),
            page: Page::Map,
            ..Default::default()
        };
        menu.focus_page();
        assert_eq!(menu.focus, Pane::Map);
        menu.page = Page::Character;
        menu.focus_page();
        assert_eq!(menu.focus, Pane::Character(0), "the first list of the page");
        menu.page = Page::Developer;
        menu.focus_page();
        assert_eq!(
            menu.focus,
            Pane::Character(0),
            "the developer page has no list"
        );
        assert!(panes(Page::Developer).is_empty());
    }
}
