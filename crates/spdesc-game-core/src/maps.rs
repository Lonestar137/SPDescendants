//! The maps a game can be played on.
//!
//! The project owns one map — the arena the practice targets stand in — and the player's Modern
//! Warfare 2 install supplies more, which `crates/spdesc-mw2` reads out of the game's own rotation.
//!
//! The rules are project-owned. A fresh game starts on the project's own map, and that map is the
//! one that carries the practice targets: an imported map is played without them, so a target is
//! something the project put there rather than something a map expects.
//!
//! A choice is saved by **id**, not by position: the install decides which maps exist, and its list
//! changes when a map is added or removed, so an index would not be stable between runs.

/// The id of the project's own arena, where the practice targets stand.
pub const DEV_MAP: &str = "dev";

/// One map that can be played.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameMap {
    /// The stable id a choice is saved as (`"dev"`, `"mp_rust"`).
    pub id: String,
    /// What the menu calls it.
    pub title: String,
    /// Whether the practice targets stand on it.
    pub targets: bool,
}

/// The maps on offer, the project's own first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Maps {
    maps: Vec<GameMap>,
}

impl Maps {
    /// Build from the project's own map and whatever the install supplied.
    ///
    /// A later map that repeats an id already held is dropped: the first statement of an id is the
    /// one the game plays, and with the project's map first that keeps its own id its own.
    pub fn new(dev: GameMap, imported: impl IntoIterator<Item = GameMap>) -> Maps {
        let mut maps = vec![dev];
        for map in imported {
            if !maps.iter().any(|held| held.id == map.id) {
                maps.push(map);
            }
        }
        Maps { maps }
    }

    pub fn len(&self) -> usize {
        self.maps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&GameMap> {
        self.maps.get(index)
    }

    pub fn maps(&self) -> &[GameMap] {
        &self.maps
    }

    /// Where the map with `id` sits in the list.
    pub fn position(&self, id: &str) -> Option<usize> {
        self.maps.iter().position(|map| map.id == id)
    }

    /// The id a fresh game starts on: the project's own map, or the first map when the project's is
    /// somehow absent. `None` only when there are no maps at all.
    pub fn default_id(&self) -> Option<&str> {
        let at = self
            .position(DEV_MAP)
            .or((!self.maps.is_empty()).then_some(0))?;
        self.maps.get(at).map(|map| map.id.as_str())
    }

    /// The map an id names, falling back to the default when the id is unknown: a saved choice for
    /// a map this install no longer ships is not an error, it is the default.
    pub fn resolve(&self, id: &str) -> Option<&GameMap> {
        let at = self
            .position(id)
            .or_else(|| self.default_id().and_then(|default| self.position(default)))?;
        self.maps.get(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(id: &str, targets: bool) -> GameMap {
        GameMap {
            id: id.to_owned(),
            title: id.to_owned(),
            targets,
        }
    }

    fn catalogue() -> Maps {
        Maps::new(
            map(DEV_MAP, true),
            [map("mp_rust", false), map("mp_terminal", false)],
        )
    }

    /// The project's map is first and is what a fresh game starts on, and it is the one whose
    /// targets stand — the rule that keeps a target something the project placed.
    #[test]
    fn the_projects_own_map_is_first_the_default_and_has_the_targets() {
        let maps = catalogue();
        assert_eq!(maps.len(), 3);
        assert_eq!(maps.get(0).map(|map| map.id.as_str()), Some(DEV_MAP));
        assert_eq!(maps.default_id(), Some(DEV_MAP));
        assert!(maps.get(0).expect("the dev map").targets);
        assert!(
            maps.maps().iter().skip(1).all(|map| !map.targets),
            "an imported map is played without the practice targets"
        );
    }

    /// A saved choice names a map by id, and an id this install does not have resolves to the
    /// default rather than to nothing.
    #[test]
    fn a_choice_resolves_by_id_and_falls_back_to_the_default() {
        let maps = catalogue();
        assert_eq!(
            maps.resolve("mp_terminal").map(|map| map.id.as_str()),
            Some("mp_terminal")
        );
        assert_eq!(maps.position("mp_rust"), Some(1));
        assert_eq!(
            maps.resolve("mp_vacant").map(|map| map.id.as_str()),
            Some(DEV_MAP),
            "a map the install does not ship is the default, not an error"
        );
        assert!(Maps::default().resolve("dev").is_none());
        assert!(Maps::default().default_id().is_none());
    }

    /// An imported map that repeats an id already held does not replace it: the project's own id
    /// stays its own, and the install cannot displace a map that is already listed.
    #[test]
    fn a_repeated_id_is_dropped() {
        let maps = Maps::new(
            map(DEV_MAP, true),
            [
                map(DEV_MAP, false),
                map("mp_rust", false),
                map("mp_rust", false),
            ],
        );
        assert_eq!(maps.len(), 2);
        assert!(maps.get(0).expect("the dev map").targets);
    }
}
