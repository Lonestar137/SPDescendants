//! Ranks, experience and prestige in the app: the game's tables, the career, and its HUD.
//!
//! The rules live in `spdesc_game_core::progression`; this module feeds them the game's own rank
//! table (`mp/ranktable.csv`), decodes the insignia art beside it, and presents the result. A hit
//! is worth [`spdesc_game_core::XP_PER_HIT`] and a kill [`spdesc_game_core::XP_PER_KILL`].
//!
//! Everything is optional in the same way the sounds are: without `MW2_DATA` there is no rank
//! table, so the career stays at the first rank with nothing to award and the HUD hides itself.

use std::collections::HashMap;
use std::path::Path;

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use spdesc_game_core::{Progression, Promotion, Rank, Ranks, XP_PER_HIT, XP_PER_KILL};
use spdesc_mw2::{ZoneData, iwd, iwi};

use crate::audio::{self, Sounds};
use crate::settings::CareerSave;

/// How long a promotion banner stays up, seconds.
const BANNER_SECONDS: f32 = 3.0;
/// What the debug grant awards, so a whole career can be walked in a session.
const DEBUG_GRANT: u32 = 25_000;

/// The rank and prestige insignia, decoded once at startup and shared.
#[derive(Resource, Default)]
pub struct Insignia {
    /// Keyed by the name the rank table gives (`rank_pvt1`, …).
    by_name: HashMap<String, Handle<Image>>,
    /// One per prestige level, index 0 being prestige 1.
    prestige: Vec<Handle<Image>>,
}

impl Insignia {
    /// The art for a rank, when the install had it.
    pub fn rank(&self, rank: &Rank) -> Option<Handle<Image>> {
        self.by_name.get(&rank.icon).cloned()
    }

    /// The art for a prestige level, when the install had it.
    pub fn prestige(&self, level: u32) -> Option<Handle<Image>> {
        (level > 0)
            .then(|| self.prestige.get(level as usize - 1).cloned())
            .flatten()
    }
}

/// The player's career: the rank table, where they stand on it, and the banner saying so.
#[derive(Resource)]
pub struct Career {
    ranks: Ranks,
    progression: Progression,
    /// Seconds left on the promotion banner, and what it says.
    banner: f32,
    banner_text: String,
    /// Experience awarded since the last frame folded it in.
    pending: u32,
    /// Whether the career has changed since it was last written, and how long until the next
    /// write. A run can end abruptly, so this writes while playing rather than only on exit.
    dirty: bool,
    save_in: f32,
}

impl Default for Career {
    fn default() -> Self {
        Career {
            ranks: Ranks::default(),
            progression: Progression::new(),
            banner: 0.0,
            banner_text: String::new(),
            pending: 0,
            dirty: false,
            save_in: 0.0,
        }
    }
}

/// How often the career is written while it is changing, seconds.
const SAVE_EVERY: f32 = 2.0;

impl Career {
    /// Build the career from the zones the app read.
    pub fn from_zones(zones: &[(String, ZoneData)]) -> Career {
        let ranks = zones
            .iter()
            .find_map(|(_, zone)| ranks_from(zone))
            .unwrap_or_default();
        Career::with_ranks(ranks)
    }

    /// A career on the given table, at its first rank — what [`from_zones`](Self::from_zones)
    /// builds, and what a test can build from a table of its own.
    pub fn with_ranks(ranks: Ranks) -> Career {
        Career {
            ranks,
            ..Career::default()
        }
    }

    /// The rank table, for a caller that wants to show it.
    pub fn ranks(&self) -> &Ranks {
        &self.ranks
    }

    pub fn progression(&self) -> &Progression {
        &self.progression
    }

    /// The rank the player holds.
    pub fn rank(&self) -> Option<&Rank> {
        self.ranks.rank(self.progression.rank_index())
    }

    /// Whether the install gave a usable rank table.
    pub fn is_loaded(&self) -> bool {
        !self.ranks.is_empty()
    }

    /// Take up a saved career, if one is there and its rank exists in the loaded table.
    pub fn restore(&mut self, save: Option<CareerSave>) {
        let Some(save) = save else {
            return;
        };
        let rank = save
            .rank_index
            .min(self.ranks.last_index().unwrap_or(0))
            .min(self.ranks.len().saturating_sub(1) as u32);
        let prestige = save.prestige.min(self.ranks.max_prestige());
        self.progression.restore(rank, prestige, save.xp_in_rank);
    }

    /// The career as it is kept between runs.
    pub fn save_data(&self) -> CareerSave {
        CareerSave {
            rank_index: self.progression.rank_index(),
            prestige: self.progression.prestige(),
            xp_in_rank: self.progression.xp_in_rank(),
        }
    }

    /// Forget the prestige and the levels: a fresh career on the loaded table.
    pub fn reset(&mut self) {
        self.progression = Progression::new();
        self.banner(BANNER_SECONDS, "PRESTIGE RESET".to_owned());
        self.dirty = true;
        self.save_in = 0.0;
    }

    /// Award experience, reporting what it did.
    pub fn award(&mut self, xp: u32) -> Vec<Promotion> {
        let events = self.progression.award(&self.ranks, xp);
        if !events.is_empty() {
            self.dirty = true;
            self.save_in = 0.0;
        }
        for event in &events {
            match event {
                Promotion::Rank { index } => {
                    let title = self
                        .ranks
                        .rank(*index)
                        .map(|r| r.title.clone())
                        .unwrap_or_default();
                    self.banner(BANNER_SECONDS, format!("LEVEL {}  {title}", index + 1));
                }
                Promotion::PrestigeReady => {
                    self.banner(BANNER_SECONDS, "PRESTIGE READY".to_owned());
                }
                Promotion::Prestige { level } => {
                    self.banner(BANNER_SECONDS, format!("PRESTIGE {level}"));
                }
            }
        }
        events
    }

    /// Prestige, when it has been earned.
    pub fn prestige(&mut self) -> bool {
        if !self.progression.prestige_up(&self.ranks) {
            return false;
        }
        let level = self.progression.prestige();
        self.banner(BANNER_SECONDS, format!("PRESTIGE {level}"));
        self.dirty = true;
        self.save_in = 0.0;
        true
    }

    /// Write the career if it has changed, at most every [`SAVE_EVERY`] seconds.
    pub fn save_if_due(&mut self, dt: f32) {
        if !self.dirty {
            return;
        }
        self.save_in -= dt;
        if self.save_in > 0.0 {
            return;
        }
        self.save_in = SAVE_EVERY;
        match self.save_data().save() {
            Ok(()) => self.dirty = false,
            // A failing write is not fatal — the career simply keeps trying.
            Err(error) => warn!("career: could not write: {error}"),
        }
    }

    fn banner(&mut self, seconds: f32, text: String) {
        self.banner = seconds;
        self.banner_text = text;
    }
}

/// Build the rank table from the game's two tables.
///
/// `mp/ranktable.csv` states, per rank: the index, the rank's name, the experience to reach it,
/// the experience to leave it, and the insignia's name. `mp/rankicontable.csv` keeps the same rank
/// order and names the insignia, with the prestige ceiling in its first data row.
fn ranks_from(zone: &ZoneData) -> Option<Ranks> {
    let table = zone.table("mp/ranktable.csv")?;
    let icons = zone.table("mp/rankicontable.csv")?;
    let max_prestige = icons
        .cell(1, 1)
        .and_then(|cell| cell.trim().parse().ok())
        .unwrap_or(0);

    let mut ranks = Vec::new();
    for row in 0..table.rows {
        let Some(index) = table
            .cell(row, 0)
            .and_then(|cell| cell.trim().parse::<u32>().ok())
        else {
            continue; // the header and the `maxrank` row are not ranks
        };
        let number = |column: usize| {
            table
                .cell(row, column)
                .and_then(|cell| cell.trim().parse::<u32>().ok())
                .unwrap_or(0)
        };
        // The icon table is indexed the same way, by the same rank index.
        let icon = (0..icons.rows)
            .find(|r| icons.cell(*r, 0) == Some(index.to_string().as_str()))
            .and_then(|r| icons.cell(r, 1))
            .unwrap_or("")
            .to_owned();
        ranks.push(Rank {
            index,
            title: table.cell(row, 1).unwrap_or("").to_owned(),
            xp_to_reach: number(2),
            xp_in_rank: number(3),
            icon,
        });
    }
    (!ranks.is_empty()).then(|| Ranks::new(ranks, max_prestige))
}

/// Decode every insignia the table names, plus each prestige level's.
pub fn load_insignia(
    install: Option<&Path>,
    career: &Career,
    images: &mut Assets<Image>,
) -> Insignia {
    let Some(install) = install else {
        return Insignia::default();
    };
    // One pass over the archives for all of them: resolving them one at a time would re-walk a
    // hundreds-of-megabytes archive for each icon.
    let mut entries: Vec<String> = Vec::new();
    for rank in career.ranks().ranks() {
        if !rank.icon.is_empty() {
            entries.push(iwd::rank_icon_entry(&rank.icon));
        }
    }
    for level in 1..=career.ranks().max_prestige() {
        entries.push(iwd::prestige_icon_entry(level));
    }
    entries.sort();
    entries.dedup();
    let files = iwd::read_many(install, &entries);

    let mut insignia = Insignia::default();
    for rank in career.ranks().ranks() {
        if rank.icon.is_empty() || insignia.by_name.contains_key(&rank.icon) {
            continue;
        }
        let entry = iwd::rank_icon_entry(&rank.icon);
        if let Some(handle) = files
            .get(&entry)
            .and_then(|bytes| decode_image(bytes, images))
        {
            insignia.by_name.insert(rank.icon.clone(), handle);
        }
    }
    for level in 1..=career.ranks().max_prestige() {
        let entry = iwd::prestige_icon_entry(level);
        if let Some(handle) = files
            .get(&entry)
            .and_then(|bytes| decode_image(bytes, images))
        {
            insignia.prestige.push(handle);
        }
    }
    insignia
}

/// An `.iwi`'s bytes as a Bevy image.
fn decode_image(bytes: &[u8], images: &mut Assets<Image>) -> Option<Handle<Image>> {
    let decoded = iwi::decode(bytes).ok()?;
    Some(images.add(Image::new(
        Extent3d {
            width: decoded.width,
            height: decoded.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        decoded.rgba,
        // The art is authored in sRGB, like the game's own textures.
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    )))
}

/// The HUD pieces the career draws into.
#[derive(Component)]
pub struct RankIcon;

#[derive(Component)]
pub struct PrestigeIcon;

#[derive(Component)]
pub struct RankText;

/// The filled part of the experience bar.
#[derive(Component)]
pub struct XpFill;

/// The promotion banner.
#[derive(Component)]
pub struct PromotionBanner;

/// The rank panel, hidden while the menu is open.
#[derive(Component)]
pub struct ProgressPanel;

/// Award the experience the sim reported, and ring the promotion cue.
///
/// The award is settled here rather than at the moment of the hit so that the sim's own event
/// stays a report; this is still one frame.
pub fn award_experience(mut commands: Commands, sounds: Res<Sounds>, mut career: ResMut<Career>) {
    if career.pending == 0 {
        return;
    }
    let xp = std::mem::take(&mut career.pending);
    // A promotion is rare and worth a line: it is the only record of a career outside the HUD.
    for event in career.award(xp) {
        match event {
            Promotion::Rank { index } => {
                info!("promotion: rank {}", index + 1);
                audio::play(&mut commands, &sounds.promotion, 0.6);
            }
            // Readiness is a message, not a fanfare; `O` takes it and rings the prestige cue.
            Promotion::PrestigeReady => info!("promotion: prestige ready"),
            Promotion::Prestige { level } => {
                info!("promotion: prestige {level}");
                audio::play(&mut commands, &sounds.prestige, 0.7);
            }
        }
    }
}

/// Award experience to the career, which the gameplay systems call when a target is hit or killed.
pub fn grant(career: &mut Career, xp: u32) {
    career.pending = career.pending.saturating_add(xp);
}

/// Keep the career on disk while it changes.
pub fn save_career(time: Res<Time>, mut career: ResMut<Career>) {
    career.save_if_due(time.delta_secs());
}

/// Write the career on the way out, so a clean exit keeps the level exactly.
pub fn save_career_on_exit(
    mut exits: MessageReader<AppExit>,
    career: Res<Career>,
    mut done: Local<bool>,
) {
    if *done || exits.read().next().is_none() {
        return;
    }
    *done = true;
    if let Err(error) = career.save_data().save() {
        warn!("career: could not write on exit: {error}");
    }
}

/// The experience a hit and a kill are worth, for the gameplay systems.
pub const HIT_XP: u32 = XP_PER_HIT;
pub const KILL_XP: u32 = XP_PER_KILL;

/// The debug keys: `P` grants experience and `O` takes the prestige that is on offer, so the whole
/// career can be walked in a session without shooting two thousand targets.
pub fn debug_progress(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    sounds: Res<Sounds>,
    mut career: ResMut<Career>,
) {
    if !career.is_loaded() {
        return;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        grant(&mut career, DEBUG_GRANT);
    }
    if keys.just_pressed(KeyCode::KeyO) && career.prestige() {
        info!("promotion: prestige {}", career.progression().prestige());
        audio::play(&mut commands, &sounds.prestige, 0.7);
    }
}

/// Draw the career: the insignia, the rank and its progress, and the promotion banner.
// Two of the queries are long because they are deliberately disjoint by marker: both write
// `Visibility`, so Bevy has to be told they never overlap. `crates/spdesc` allows a long
// parameter list for the same reason (`too_many_arguments` in the workspace lints).
#[allow(clippy::type_complexity)]
pub fn draw_progress(
    menu: Res<crate::Menu>,
    time: Res<Time>,
    career: ResMut<Career>,
    insignia: Res<Insignia>,
    // `Without<RankIcon>` keeps this disjoint from the icon query below: both write `Visibility`,
    // and Bevy refuses two mutable accesses it cannot prove never overlap.
    mut panels: Query<&mut Visibility, (With<ProgressPanel>, Without<RankIcon>)>,
    mut texts: Query<(
        &mut Text,
        &mut TextColor,
        Option<&RankText>,
        Option<&PromotionBanner>,
    )>,
    mut fills: Query<&mut Node, With<XpFill>>,
    mut rank_icons: Query<
        (&mut ImageNode, &mut Visibility),
        (With<RankIcon>, Without<PrestigeIcon>),
    >,
    mut prestige_icons: Query<&mut ImageNode, With<PrestigeIcon>>,
) {
    let career = career.into_inner();
    let panel_visibility = if menu.open || !career.is_loaded() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut panel in &mut panels {
        if *panel != panel_visibility {
            *panel = panel_visibility;
        }
    }
    if career.banner > 0.0 {
        career.banner = (career.banner - time.delta_secs()).max(0.0);
    }
    let progress = career.progression().progress(career.ranks());
    let rank = career.rank().cloned();
    let prestige = career.progression().prestige();

    for mut fill in &mut fills {
        fill.width = Val::Percent(progress * 100.0);
    }
    for (mut node, mut visibility) in &mut rank_icons {
        match rank.as_ref().and_then(|rank| insignia.rank(rank)) {
            Some(handle) => {
                node.image = handle;
                *visibility = Visibility::Inherited;
            }
            None => *visibility = Visibility::Hidden,
        }
    }
    for mut icon in &mut prestige_icons {
        if let Some(handle) = insignia.prestige(prestige) {
            icon.image = handle;
        }
    }
    for (mut text, mut colour, rank_text, banner) in &mut texts {
        if banner.is_some() {
            let rendered = if career.banner > 0.0 {
                career.banner_text.clone()
            } else {
                String::new()
            };
            if text.0 != rendered {
                text.0 = rendered;
            }
            // Fade out over the last second.
            let alpha = if career.banner > 1.0 {
                1.0
            } else {
                career.banner
            };
            *colour = TextColor(Color::srgba(1.0, 0.86, 0.42, alpha));
            continue;
        }
        if rank_text.is_some() {
            let rendered = match &rank {
                Some(rank) => {
                    let level = rank.index + 1;
                    let total = career
                        .ranks()
                        .last_index()
                        .map(|last| last + 1)
                        .unwrap_or(0);
                    if prestige > 0 {
                        format!("{level}/{total}   PRESTIGE {prestige}")
                    } else {
                        format!("{level}/{total}")
                    }
                }
                None => String::new(),
            };
            if text.0 != rendered {
                text.0 = rendered;
            }
        }
    }
}

/// The promotion banner's node, and the panel the career draws into.
pub fn setup_progress(mut commands: Commands) {
    // The rank panel, bottom-left: insignia, rank, and the experience bar.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(28.0),
                bottom: Val::Px(22.0),
                column_gap: Val::Px(10.0),
                align_items: AlignItems::Center,
                ..default()
            },
            ProgressPanel,
            Visibility::Inherited,
        ))
        .with_children(|parent| {
            parent.spawn((
                RankIcon,
                ImageNode::default(),
                Node {
                    width: Val::Px(48.0),
                    height: Val::Px(48.0),
                    ..default()
                },
            ));
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|parent| {
                    parent.spawn((
                        RankText,
                        Text::new(""),
                        TextFont {
                            font_size: bevy::text::FontSize::Px(18.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    ));
                    // The bar: an empty track with a fill in front of it.
                    parent
                        .spawn((
                            Node {
                                width: Val::Px(160.0),
                                height: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.18)),
                        ))
                        .with_children(|parent| {
                            parent.spawn((
                                XpFill,
                                Node {
                                    width: Val::Percent(0.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.86, 0.78, 0.42)),
                            ));
                        });
                });
            parent.spawn((
                PrestigeIcon,
                ImageNode::default(),
                Node {
                    width: Val::Px(52.0),
                    height: Val::Px(52.0),
                    margin: UiRect::left(Val::Px(6.0)),
                    ..default()
                },
                Visibility::Hidden,
            ));
        });

    // The promotion banner, across the middle of the screen.
    commands.spawn((
        PromotionBanner,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(46.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 0.86, 0.42, 0.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(22.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::CareerSave;

    /// A small table in the game's shape: five ranks, two prestige levels.
    fn table_career() -> Career {
        Career::with_ranks(Ranks::new(
            (0..5)
                .map(|index| Rank {
                    index,
                    title: format!("r{index}"),
                    xp_to_reach: index * 1_000,
                    xp_in_rank: 1_000,
                    icon: String::new(),
                })
                .collect(),
            2,
        ))
    }

    /// A career goes to disk and comes back: the level, the prestige and the experience inside the
    /// rank all survive, which is what "the level persists" means.
    #[test]
    fn a_career_round_trips_through_its_saved_form() {
        let mut career = table_career();
        career.restore(Some(CareerSave {
            rank_index: 4,
            prestige: 2,
            xp_in_rank: 7_00,
        }));
        let save = career.save_data();
        assert_eq!(
            save,
            CareerSave {
                rank_index: 4,
                prestige: 2,
                xp_in_rank: 700
            }
        );
        let mut restored = table_career();
        restored.restore(CareerSave::from_json(&save.to_json()));
        assert_eq!(restored.save_data(), save);
    }

    /// A saved career cannot put the player somewhere the table cannot draw: a rank past the end
    /// or a prestige past the ceiling is clamped, and an empty table holds everything at zero.
    #[test]
    fn a_career_cannot_restore_past_the_table() {
        let mut career = table_career();
        career.restore(Some(CareerSave {
            rank_index: 999,
            prestige: 999,
            xp_in_rank: 10,
        }));
        assert_eq!(career.progression().rank_index(), 4, "the last rank");
        assert_eq!(career.progression().prestige(), 2, "the ceiling");
        assert_eq!(career.progression().xp_in_rank(), 10);

        let mut empty = Career::default();
        empty.restore(Some(CareerSave {
            rank_index: 3,
            prestige: 1,
            xp_in_rank: 10,
        }));
        assert_eq!(
            (
                empty.progression().rank_index(),
                empty.progression().prestige()
            ),
            (0, 0),
            "with no table there is nowhere to stand"
        );
    }

    /// Resetting, which the dev menu's button does, forgets the prestige and the levels.
    #[test]
    fn resetting_forgets_the_prestige_and_the_levels() {
        let mut career = table_career();
        career.restore(Some(CareerSave {
            rank_index: 3,
            prestige: 2,
            xp_in_rank: 400,
        }));
        career.reset();
        assert_eq!(
            career.save_data(),
            CareerSave {
                rank_index: 0,
                prestige: 0,
                xp_in_rank: 0
            }
        );
        assert_eq!(career.banner_text, "PRESTIGE RESET");
    }

    /// The career reads the game's own rank table, and the experience the targets pay walks it:
    /// the thresholds, the rank names and the insignia come from the install rather than from a
    /// table kept here, so this is what keeps them honest.
    #[test]
    fn the_career_comes_from_the_games_rank_table() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let bank = spdesc_mw2::SoundBank::read(&install, &["code_post_gfx_mp"]);
        let mut career = Career::from_zones(bank.zones());
        assert!(
            career.is_loaded(),
            "no rank table in code_post_gfx_mp: {:?}",
            bank.failures()
        );
        // The table is cloned out: awarding borrows the career mutably and the table immutably.
        let ranks = career.ranks().clone();
        assert_eq!(ranks.len(), 70, "Modern Warfare 2's ranks");
        assert_eq!(ranks.max_prestige(), 10);

        let first = ranks.rank(0).expect("the first rank").clone();
        assert_eq!(first.title, "pvt1");
        assert_eq!((first.xp_to_reach, first.xp_in_rank), (0, 500));
        for rank in ranks.ranks() {
            assert!(
                !rank.icon.is_empty(),
                "rank {} ({}) names no insignia",
                rank.index,
                rank.title
            );
        }

        // Shoot one's way out of the first rank: hits alone should do it at the table's numbers.
        let hits = first.xp_in_rank.div_ceil(spdesc_game_core::XP_PER_HIT);
        let mut events = Vec::new();
        for _ in 0..hits {
            events.extend(career.award(spdesc_game_core::XP_PER_HIT));
        }
        assert_eq!(
            events,
            vec![Promotion::Rank { index: 1 }],
            "the first rank falls after {hits} hits"
        );
        assert_eq!(career.progression().rank_index(), 1);

        // A target that goes down is worth a kill's, and a full career reaches prestige.
        let last = ranks.last_index().expect("a last rank");
        let mut career = Career::from_zones(bank.zones());
        let mut promotions = Vec::new();
        let mut guard = 0;
        while career.progression().rank_index() < last && guard < 1_000_000 {
            promotions.extend(career.award(spdesc_game_core::XP_PER_KILL));
            guard += 1;
        }
        assert_eq!(
            career.progression().rank_index(),
            last,
            "kills alone reach the last rank"
        );
        assert!(
            promotions
                .iter()
                .any(|p| matches!(p, Promotion::Rank { index } if *index == last)),
            "and the last promotion is reported"
        );
    }
}
