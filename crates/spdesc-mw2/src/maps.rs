//! The multiplayer maps an install ships, read from the game's own arena file.
//!
//! `mp/basemaps.arena`, inside `code_post_gfx_mp.ff`, is the game's map rotation: one block per map
//! with the map's own name (which is also its fastfile — `mp_rust` is `zone/english/mp_rust.ff`),
//! the localisation key it is titled with, the gametypes it offers, the preview art it is drawn
//! with, its environment tag, and the two factions that fight there.
//!
//! This reads the file and nothing else: no map geometry is parsed, so a map read here is a map
//! *named* — the app decides what to present for it. The blocks are indented with tabs in some
//! places and spaces in others and their values are inconsistently quoted, so the reader is a token
//! walk rather than a column parse.

use std::path::Path;

use crate::sounds::ZoneData;

/// One multiplayer map, as the game's arena file describes it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MapEntry {
    /// The map's own name, which is also its fastfile under `zone/english`.
    pub id: String,
    /// The localisation key the game titles it with (`MPUI_RAID`, `PATCH_VACANT`). The key is not
    /// the title — the title is whatever the game's localisation says — but it names the map where
    /// the map's own name would not (`mp_nightshift` is keyed `MPUI_RAID`).
    pub title_key: String,
    /// The art's name for the map's preview, `images/<preview>.iwi` in the archives.
    pub preview: String,
    /// The gametype codes the map offers, in the game's own order.
    pub gametypes: Vec<String>,
    /// The art team's environment tag (`urban`, `desert`, …).
    pub environment: String,
    /// The two factions, by character-set name.
    pub allies: String,
    pub axis: String,
}

/// One token of an arena file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token<'a> {
    /// A key, or a value the file left unquoted.
    Word(&'a str),
    /// A value the file quoted.
    Text(&'a str),
    Open,
    Close,
}

/// Split an arena file into tokens: `{` and `}` stand alone, a quoted run is one token, and
/// anything else is a run of non-space characters. `//` runs to the end of its line.
fn tokens(text: &str) -> Vec<Token<'_>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if byte == b'/' && bytes.get(at + 1) == Some(&b'/') {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        match byte {
            b'{' => {
                out.push(Token::Open);
                at += 1;
            }
            b'}' => {
                out.push(Token::Close);
                at += 1;
            }
            b'"' => {
                let start = at + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += 1;
                }
                out.push(Token::Text(&text[start..end]));
                at = (end + 1).min(bytes.len());
            }
            _ => {
                let start = at;
                while at < bytes.len()
                    && !bytes[at].is_ascii_whitespace()
                    && !matches!(bytes[at], b'{' | b'}' | b'"')
                {
                    at += 1;
                }
                out.push(Token::Word(&text[start..at]));
            }
        }
    }
    out
}

/// A block being read: every field optional until its `map` line names it.
#[derive(Default)]
struct Block {
    entry: MapEntry,
    named: bool,
}

impl Block {
    fn set(&mut self, key: &str, value: &str) {
        match key {
            "map" => {
                self.entry.id = value.to_owned();
                self.named = true;
            }
            "longname" => self.entry.title_key = value.to_owned(),
            "mapimage" => self.entry.preview = value.to_owned(),
            "gametype" => {
                self.entry.gametypes = value.split_whitespace().map(str::to_owned).collect()
            }
            "environment" => self.entry.environment = value.to_owned(),
            "allieschar" => self.entry.allies = value.to_owned(),
            "axischar" => self.entry.axis = value.to_owned(),
            // Everything else the file carries — descriptions, overlays, and whatever a later
            // revision adds — is not read.
            _ => {}
        }
    }
}

/// Parse an arena file into its maps, in the order the file states them.
///
/// A block without a `map` line is not a map and is skipped, and a block left open at the end of
/// the file is dropped rather than invented.
pub fn parse_arena(text: &str) -> Vec<MapEntry> {
    let mut maps = Vec::new();
    let mut block: Option<Block> = None;
    let mut key: Option<String> = None;
    for token in tokens(text) {
        match token {
            Token::Open => {
                block = Some(Block::default());
                key = None;
            }
            Token::Close => {
                if let Some(block) = block.take()
                    && block.named
                {
                    maps.push(block.entry);
                }
                key = None;
            }
            // A word is a value when a key is waiting for one, and a key otherwise.
            Token::Word(word) => match (block.as_mut(), key.take()) {
                (Some(block), Some(key)) => block.set(&key, word),
                _ => key = Some(word.to_ascii_lowercase()),
            },
            Token::Text(value) => {
                if let (Some(block), Some(key)) = (block.as_mut(), key.take()) {
                    block.set(&key, value);
                }
            }
        }
    }
    maps
}

/// The archive entry a map's preview art lives at, given the name the arena file uses.
pub fn preview_entry(preview: &str) -> String {
    format!("images/{preview}.iwi")
}

/// The maps `zone` carries, or none when it is not the zone that holds the arena file.
pub fn maps_in(zone: &ZoneData) -> Vec<MapEntry> {
    let Some(file) = zone.raw_file("mp/basemaps.arena") else {
        return Vec::new();
    };
    let Ok(bytes) = file.data() else {
        return Vec::new();
    };
    parse_arena(&String::from_utf8_lossy(&bytes))
}

/// The map list in the first zone that carries one.
pub fn read_maps(zones: &[(String, ZoneData)]) -> Vec<MapEntry> {
    zones
        .iter()
        .map(|(_, zone)| maps_in(zone))
        .find(|maps| !maps.is_empty())
        .unwrap_or_default()
}

/// Where a map's own fastfile sits, which is also the check for whether the install ships it: an
/// arena file lists downloadable maps the install may not have.
pub fn map_zone_path(install: &Path, id: &str) -> std::path::PathBuf {
    crate::zone_path(install, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The blocks are indented with tabs in places and spaces in others, and quoting is not
    /// consistent, so this is the same shape as the game's own file with nothing copied from it.
    const MIXED: &str = r#"
// The base rotation.
{
	map			"mp_alpha"
	longname	"MPUI_ALPHA"
	gametype	"dm war sab dom"
    mapimage    "preview_mp_alpha"
	mapoverlay  "compass_overlay_map_blank"
    allieschar  "us_army"
    axischar    "opforce_airborne"
    environment	"urban"
}

{
    map		mp_beta
    longname	PATCH_BETA
    gametype	"tdm koth"
    mapimage	"preview_mp_beta"
    environment	"desert"
}
"#;

    /// Every field of every block is read, whatever the indentation or the quoting, and the order
    /// the file states is the order they come back in.
    #[test]
    fn a_mixed_arena_file_reads_field_by_field() {
        let maps = parse_arena(MIXED);
        assert_eq!(maps.len(), 2);

        let alpha = &maps[0];
        assert_eq!(alpha.id, "mp_alpha");
        assert_eq!(alpha.title_key, "MPUI_ALPHA");
        assert_eq!(alpha.preview, "preview_mp_alpha");
        assert_eq!(alpha.gametypes, ["dm", "war", "sab", "dom"]);
        assert_eq!(alpha.environment, "urban");
        assert_eq!(alpha.allies, "us_army");
        assert_eq!(alpha.axis, "opforce_airborne");

        // Unquoted values read the same as quoted ones.
        let beta = &maps[1];
        assert_eq!(beta.id, "mp_beta");
        assert_eq!(beta.title_key, "PATCH_BETA");
        assert_eq!(beta.gametypes, ["tdm", "koth"]);
        assert_eq!(beta.environment, "desert");
        assert_eq!(beta.allies, "", "a block without factions says so");
    }

    /// A block that never names a map is not a map, and a block left open at the end of the file is
    /// dropped rather than invented.
    #[test]
    fn blocks_without_a_map_and_unterminated_blocks_are_dropped() {
        let maps = parse_arena(
            r#"
{ longname "MPUI_X" mapimage "preview_mp_x" }
{ map "mp_real" gametype "dm" }
{ map "mp_cut_off" gametype "dm"
"#,
        );
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].id, "mp_real");
        assert!(parse_arena("").is_empty());
    }

    /// The preview art is named after the file's own map image name.
    #[test]
    fn the_preview_entry_is_named_after_the_map_image() {
        assert_eq!(
            preview_entry("preview_mp_rust"),
            "images/preview_mp_rust.iwi"
        );
    }

    /// The install's own arena file reads, and every map it names has the fields the menu needs.
    /// The maps whose zone the install does not ship are listed too — the arena file is the game's
    /// rotation, not the install's contents — which is why the caller filters by [`map_zone_path`].
    #[test]
    fn the_install_ships_a_map_list_the_menu_can_use() {
        let Some(install) = spdesc_game_core::mw2_data() else {
            spdesc_game_core::MW2.skip("no MW2_DATA folder found");
            return;
        };
        let bank = crate::SoundBank::read(&install, &["code_post_gfx_mp"]);
        let maps = bank.maps();
        assert!(maps.len() > 20, "the arena file lists {} maps", maps.len());
        for map in &maps {
            assert!(map.id.starts_with("mp_"), "{} is not a map name", map.id);
            assert!(!map.preview.is_empty(), "{} has no preview", map.id);
            assert!(!map.gametypes.is_empty(), "{} has no gametypes", map.id);
        }
        // The map's name is its fastfile, and the install ships at least a few of them.
        let shipped: Vec<&str> = maps
            .iter()
            .filter(|map| map_zone_path(&install, &map.id).is_file())
            .map(|map| map.id.as_str())
            .collect();
        assert!(
            shipped.len() > 10,
            "the install ships {} of the {} listed maps",
            shipped.len(),
            maps.len()
        );
        // Every shipped map has its preview art too.
        let entries: Vec<String> = shipped
            .iter()
            .map(|id| {
                let preview = maps
                    .iter()
                    .find(|map| map.id == *id)
                    .map(|map| map.preview.as_str())
                    .unwrap_or_default();
                preview_entry(preview)
            })
            .collect();
        let found = crate::iwd::read_many(&install, &entries);
        assert_eq!(
            found.len(),
            entries.len(),
            "some shipped map's preview is not in the archives"
        );
    }
}
