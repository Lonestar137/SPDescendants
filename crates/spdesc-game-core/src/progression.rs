//! Ranks, experience and prestige.
//!
//! The rules are project-owned; the *numbers* are the game's. Modern Warfare 2 ships its rank
//! table as `mp/ranktable.csv` (`crates/spdesc-mw2` reads it out of the zone), and
//! [`Ranks::new`] takes those rows: one per rank, with the total experience needed to reach it and
//! the experience it takes to leave it. `mp/rankicontable.csv` names each rank's insignia, and
//! the ceiling `maxprestige | 10` is [`Ranks::max_prestige`].
//!
//! Nothing here reads a clock or a random number: an award is a pure function of the experience
//! added, so a test can drive a whole career in one call.

/// What a target is worth, project-owned content: a hit pays a tenth of a kill, and a kill pays
/// what Modern Warfare 2's team deathmatch pays.
pub const XP_PER_HIT: u32 = 10;
pub const XP_PER_KILL: u32 = 100;

/// One rank, as the game's rank table states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rank {
    /// The rank index, counting from zero — the table's first column.
    pub index: u32,
    /// The rank's name in the table (`pvt1`, `sgtmaj1`, …).
    pub title: String,
    /// Total experience needed to reach this rank.
    pub xp_to_reach: u32,
    /// Experience needed *inside* this rank to reach the next one.
    pub xp_in_rank: u32,
    /// The name of the rank's insignia art, which the icon table resolves.
    pub icon: String,
}

/// The ranks a player can climb, and the prestige ceiling above them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranks {
    ranks: Vec<Rank>,
    max_prestige: u32,
}

impl Ranks {
    /// Build from the game's rank rows, in rank order.
    ///
    /// A table with no rows is empty, and a rank with no table entries is unreachable — every
    /// query below answers `None` rather than guessing.
    pub fn new(mut ranks: Vec<Rank>, max_prestige: u32) -> Ranks {
        ranks.sort_by_key(|r| r.index);
        Ranks {
            ranks,
            max_prestige,
        }
    }

    /// How many ranks the table holds.
    pub fn len(&self) -> usize {
        self.ranks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranks.is_empty()
    }

    /// The highest prestige the table allows.
    pub fn max_prestige(&self) -> u32 {
        self.max_prestige
    }

    /// The last rank, which is where prestige becomes available.
    pub fn last_index(&self) -> Option<u32> {
        self.ranks.last().map(|r| r.index)
    }

    /// The rank at `index`.
    pub fn rank(&self, index: u32) -> Option<&Rank> {
        self.ranks.iter().find(|r| r.index == index)
    }

    pub fn ranks(&self) -> &[Rank] {
        &self.ranks
    }
}

/// What an award did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Promotion {
    /// The player reached the rank at this index.
    Rank { index: u32 },
    /// The last rank has been held for a full rank's worth of experience: prestige is available.
    PrestigeReady,
    /// The player prestiged, and is back at the first rank at this prestige.
    Prestige { level: u32 },
}

/// A player's career: the rank held, the prestige reached, and the experience inside the rank.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progression {
    rank_index: u32,
    prestige: u32,
    xp_in_rank: u32,
    /// Set once the last rank has been held long enough; cleared by prestiging.
    prestige_ready: bool,
}

impl Progression {
    /// A career at the first rank, with no prestige.
    pub fn new() -> Progression {
        Progression::default()
    }

    pub fn rank_index(&self) -> u32 {
        self.rank_index
    }

    pub fn prestige(&self) -> u32 {
        self.prestige
    }

    /// Experience earned inside the current rank.
    pub fn xp_in_rank(&self) -> u32 {
        self.xp_in_rank
    }

    /// The experience the current rank needs to be left, or `None` at the last rank.
    pub fn xp_for_next_rank(&self, ranks: &Ranks) -> Option<u32> {
        let rank = ranks.rank(self.rank_index)?;
        (self.rank_index < ranks.last_index()?).then_some(rank.xp_in_rank)
    }

    /// How far through the current rank the career is, between 0.0 and 1.0.
    ///
    /// At the last rank this is the progress towards prestige, which is the same amount of
    /// experience again.
    pub fn progress(&self, ranks: &Ranks) -> f32 {
        match self.xp_for_next_rank(ranks) {
            Some(needed) if needed > 0 => (self.xp_in_rank as f32 / needed as f32).min(1.0),
            _ => 0.0,
        }
    }

    /// Add experience, promoting as far as it reaches.
    ///
    /// Several ranks can fall to one award, and the promotions come back in the order they were
    /// reached. At the last rank the experience collects towards `PrestigeReady` instead of
    /// overflowing the rank.
    pub fn award(&mut self, ranks: &Ranks, xp: u32) -> Vec<Promotion> {
        let mut events = Vec::new();
        if ranks.is_empty() {
            return events;
        }
        let Some(last) = ranks.last_index() else {
            return events;
        };
        self.xp_in_rank = self.xp_in_rank.saturating_add(xp);

        while self.rank_index < last {
            let Some(rank) = ranks.rank(self.rank_index) else {
                break;
            };
            let needed = rank.xp_in_rank;
            if needed == 0 || self.xp_in_rank < needed {
                break;
            }
            self.xp_in_rank -= needed;
            self.rank_index += 1;
            events.push(Promotion::Rank {
                index: self.rank_index,
            });
        }

        // At the last rank the same experience again buys prestige.
        if self.rank_index == last
            && self.prestige < ranks.max_prestige()
            && let Some(rank) = ranks.rank(last)
            && self.xp_in_rank >= rank.xp_in_rank
            && rank.xp_in_rank > 0
            && !self.prestige_ready
        {
            self.prestige_ready = true;
            events.push(Promotion::PrestigeReady);
        }
        events
    }

    /// Whether the last rank has been held long enough to prestige.
    ///
    /// Prestiging is the player's choice — the game asks for it from a menu — so this reports
    /// readiness and [`prestige_up`](Self::prestige_up) acts on it.
    pub fn can_prestige(&self, ranks: &Ranks) -> bool {
        self.prestige_ready
            && self.prestige < ranks.max_prestige()
            && ranks.last_index() == Some(self.rank_index)
    }

    /// Prestige: the rank goes back to the first and the experience with it, as the game does.
    /// Returns whether it happened.
    pub fn prestige_up(&mut self, ranks: &Ranks) -> bool {
        if !self.can_prestige(ranks) {
            return false;
        }
        self.prestige += 1;
        self.rank_index = 0;
        self.xp_in_rank = 0;
        self.prestige_ready = false;
        true
    }

    /// The rank a career at `rank_index` and `prestige` stands at, for restoring a save.
    pub fn restore(&mut self, rank_index: u32, prestige: u32, xp_in_rank: u32) {
        self.rank_index = rank_index;
        self.prestige = prestige;
        self.xp_in_rank = xp_in_rank;
        self.prestige_ready = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small table in the game's shape: three ranks, 100 experience each to leave.
    fn ranks() -> Ranks {
        Ranks::new(
            (0..3)
                .map(|index| Rank {
                    index,
                    title: format!("r{index}"),
                    xp_to_reach: index * 100,
                    xp_in_rank: 100,
                    icon: format!("rank_{index}"),
                })
                .collect(),
            2,
        )
    }

    #[test]
    fn experience_below_the_rank_requirement_promotes_nothing() {
        let mut career = Progression::new();
        assert_eq!(career.award(&ranks(), 99), vec![]);
        assert_eq!(career.rank_index(), 0);
        assert_eq!(career.xp_in_rank(), 99);
    }

    #[test]
    fn an_award_that_reaches_the_requirement_promotes_once_and_carries_the_rest() {
        let mut career = Progression::new();
        assert_eq!(
            career.award(&ranks(), 150),
            vec![Promotion::Rank { index: 1 }]
        );
        assert_eq!((career.rank_index(), career.xp_in_rank()), (1, 50));
    }

    #[test]
    fn one_award_can_cross_several_ranks_in_order() {
        let mut career = Progression::new();
        assert_eq!(
            career.award(&ranks(), 250),
            vec![Promotion::Rank { index: 1 }, Promotion::Rank { index: 2 }],
            "each rank reached is reported once, in order"
        );
        assert_eq!((career.rank_index(), career.xp_in_rank()), (2, 50));
    }

    #[test]
    fn the_last_rank_collects_toward_prestige_and_offers_it_once() {
        let mut career = Progression::new();
        let ranks = ranks();
        career.award(&ranks, 200);
        assert_eq!(career.rank_index(), 2, "at the last rank");
        assert!(!career.can_prestige(&ranks), "not held long enough yet");

        let events = career.award(&ranks, 100);
        assert_eq!(events, vec![Promotion::PrestigeReady]);
        assert!(career.can_prestige(&ranks));
        assert_eq!(
            career.award(&ranks, 500),
            vec![],
            "and not offered again every time experience arrives"
        );
    }

    #[test]
    fn prestiging_returns_to_the_first_rank_and_raises_the_level() {
        let mut career = Progression::new();
        let ranks = ranks();
        career.award(&ranks, 300);
        assert!(career.prestige_up(&ranks));
        assert_eq!((career.rank_index(), career.xp_in_rank()), (0, 0));
        assert_eq!(career.prestige(), 1);
        assert!(!career.can_prestige(&ranks), "and it must be earned again");
    }

    #[test]
    fn prestige_stops_at_the_ceiling_the_table_gives() {
        let mut career = Progression::new();
        let ranks = ranks();
        for level in 1..=2 {
            career.award(&ranks, 300);
            assert!(career.prestige_up(&ranks), "prestige {level}");
        }
        assert_eq!(career.prestige(), 2, "the table's ceiling");
        career.award(&ranks, 300);
        assert!(!career.prestige_up(&ranks), "and no further");
        assert_eq!(
            career.award(&ranks, 300),
            vec![],
            "the last prestige offers nothing more"
        );
    }

    #[test]
    fn progress_runs_from_zero_to_one_within_a_rank() {
        let mut career = Progression::new();
        let ranks = ranks();
        assert_eq!(career.progress(&ranks), 0.0);
        career.award(&ranks, 50);
        assert!((career.progress(&ranks) - 0.5).abs() < f32::EPSILON);
        career.award(&ranks, 50);
        assert_eq!(career.progress(&ranks), 0.0, "a new rank starts empty");
    }

    #[test]
    fn restoring_a_career_puts_it_back_where_it_was() {
        let mut career = Progression::new();
        career.restore(17, 3, 42);
        assert_eq!(
            (career.rank_index(), career.prestige(), career.xp_in_rank()),
            (17, 3, 42)
        );
        assert!(
            !career.can_prestige(&ranks()),
            "a restored career has not earned it"
        );
    }

    #[test]
    fn an_empty_table_is_inert_rather_than_a_panic() {
        let empty = Ranks::new(Vec::new(), 10);
        let mut career = Progression::new();
        assert!(empty.is_empty());
        assert_eq!(career.award(&empty, 1_000_000), vec![]);
        assert_eq!(career.rank_index(), 0);
        assert!(!career.prestige_up(&empty));
        assert_eq!(career.progress(&empty), 0.0);
    }
}
