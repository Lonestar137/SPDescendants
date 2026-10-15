//! The sound assets of a Modern Warfare 2 zone: its alias lists and the audio they name.
//!
//! A zone's asset bodies are laid out one after another, and an asset's pointers are only
//! meaningful once everything before it has been loaded — so the sounds cannot be read out of the
//! image directly. This walks the zone the way the game does (`load_asset_at_observed` per asset,
//! in table order) and captures two things as they go by:
//!
//! * a [`LoadedSound`] — an audio payload stored inside the zone, captured when its asset is
//!   loaded;
//! * a [`SoundAlias`] — one variant of a sound's alias list: the name the game asks for, how it
//!   is mixed (looping, volume, pitch, distance), and which payload or streamed file it names.
//!
//! MW2 PC fastfiles are the 64-bit wire layout. The walk stops at the first asset whose type this
//! vendored reader has no layout for, which is why [`SoundSet::stopped_at`] reports where it
//! stopped: everything before it was captured regardless.

use std::collections::HashMap;

use asset_iw4::size as sz;
use asset_iw4::snd_alias::{
    SND_ALIAS_ALIAS_NAME, SND_ALIAS_DIST_MAX, SND_ALIAS_DIST_MIN, SND_ALIAS_FLAG_LOOPING,
    SND_ALIAS_FLAGS, SND_ALIAS_PITCH_MAX, SND_ALIAS_PITCH_MIN, SND_ALIAS_PROBABILITY,
    SND_ALIAS_SEQUENCE, SND_ALIAS_SOUND_FILE, SND_ALIAS_VOL_MAX, SND_ALIAS_VOL_MIN,
};
use fastfile_iw4::{
    AssetLinkSink, AssetType, Ptr, XFILE_BLOCK_VIRTUAL, ZonePtr, ZoneStream,
    load_asset_at_observed, open_asset_table,
};

use crate::open::{Error, ZoneImage};

/// One alias: a name the game asks to play, and how it is mixed.
#[derive(Clone, Debug)]
pub struct SoundAlias {
    /// The alias name, as the game's code and animations spell it.
    pub name: String,
    /// The alias list this variant belongs to (a list holds a name's variants).
    pub list: String,
    /// The payload in the zone this variant uses, when it is a loaded sound.
    pub loaded: Option<String>,
    /// The `(directory, file)` a streamed variant reads from the install's `.iwd` archives.
    pub streamed: Option<(String, String)>,
    /// Whether the alias is authored to loop.
    pub looping: bool,
    /// Relative volume and pitch, and the distance window, as authored.
    pub volume: [f32; 2],
    pub pitch: [f32; 2],
    pub distance: [f32; 2],
    /// Chance this variant is the one played, and its order in the list.
    pub probability: f32,
    pub sequence: i32,
}

/// An audio payload stored inside a zone.
#[derive(Clone)]
pub struct LoadedSound {
    /// The payload's own name, which an alias's `loaded` field matches.
    pub name: String,
    /// The game's sound format code; 1 is uncompressed PCM.
    pub format: i32,
    pub rate: u32,
    pub bits: i32,
    pub channels: i32,
    pub block_size: u32,
    /// The encoded bytes exactly as the zone stores them.
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for LoadedSound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedSound")
            .field("name", &self.name)
            .field("format", &self.format)
            .field("rate", &self.rate)
            .field("bits", &self.bits)
            .field("channels", &self.channels)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

impl LoadedSound {
    /// Whether the payload is uncompressed 16-bit PCM, which needs no decoder.
    pub fn is_pcm(&self) -> bool {
        self.format == PCM && self.bits == 16 && self.channels > 0 && self.rate > 0
    }

    /// The payload's length in seconds, when it is 16-bit PCM.
    pub fn duration_seconds(&self) -> Option<f64> {
        if !self.is_pcm() {
            return None;
        }
        let frame = 2 * self.channels as u64; // two bytes per sample
        Some(self.bytes.len() as f64 / (frame * self.rate as u64) as f64)
    }

    /// The payload as a mono or stereo 16-bit WAV file.
    ///
    /// Bevy's `AudioSource` decodes WAV (the `wav` feature is on), so this is the whole bridge
    /// from a zone payload to something that plays: `AudioSource { bytes: to_wav()?.into() }`.
    pub fn to_wav(&self) -> Option<Vec<u8>> {
        if !self.is_pcm() {
            return None;
        }
        let channels = self.channels as u16;
        let rate = self.rate;
        let data_len = self.bytes.len() as u32;
        let mut out = Vec::with_capacity(44 + self.bytes.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&(channels * 2).to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.extend_from_slice(&self.bytes);
        Some(out)
    }
}

/// The format code the game uses for uncompressed PCM.
pub const PCM: i32 = 1;

/// One of the game's string tables: what a `.csv` in the install holds.
#[derive(Clone, Debug, Default)]
pub struct Table {
    /// The table's name, e.g. `mp/rankIconTable.csv`.
    pub name: String,
    pub columns: usize,
    pub rows: usize,
    /// The cells, row by row, `columns` each.
    pub cells: Vec<String>,
}

impl Table {
    /// One cell, or `None` when it is outside the table.
    pub fn cell(&self, row: usize, column: usize) -> Option<&str> {
        if row >= self.rows || column >= self.columns {
            return None;
        }
        self.cells
            .get(row * self.columns + column)
            .map(String::as_str)
    }

    /// The column a header names, so a table can be read by name rather than by index.
    pub fn column(&self, header: &str) -> Option<usize> {
        (0..self.columns).find(|c| {
            self.cell(0, *c)
                .is_some_and(|cell| cell.eq_ignore_ascii_case(header))
        })
    }
}

/// A raw file inside a zone: a `.csv`, `.gsc` or other data blob the game ships.
#[derive(Clone)]
pub struct RawFile {
    pub name: String,
    /// The bytes as the zone stores them.
    pub bytes: Vec<u8>,
    /// Whether those bytes are a zlib stream rather than the file itself.
    pub zlib_compressed: bool,
}

impl RawFile {
    /// The file's bytes, inflating them when the zone stored them compressed.
    pub fn data(&self) -> Result<Vec<u8>, Error> {
        if !self.zlib_compressed {
            return Ok(self.bytes.clone());
        }
        crate::open::inflate_zlib(&self.bytes)
    }
}

impl std::fmt::Debug for RawFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawFile")
            .field("name", &self.name)
            .field("bytes", &self.bytes.len())
            .field("zlib", &self.zlib_compressed)
            .finish()
    }
}

/// Everything a zone's walk captured.
#[derive(Default)]
pub struct ZoneData {
    pub aliases: Vec<SoundAlias>,
    pub loaded: Vec<LoadedSound>,
    /// The game's string tables, by name.
    pub tables: Vec<Table>,
    /// The game's raw files, by name.
    pub raw_files: Vec<RawFile>,
    /// How many asset bodies the walk loaded before it stopped or finished.
    pub walked: usize,
    /// The asset the walk stopped on and why, when it could not finish the zone.
    pub stopped_at: Option<String>,
}

impl ZoneData {
    /// A string table by name.
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    /// A raw file by name.
    pub fn raw_file(&self, name: &str) -> Option<&RawFile> {
        self.raw_files.iter().find(|f| f.name == name)
    }

    /// Every alias whose name matches `predicate`.
    pub fn aliases_matching<'s>(
        &'s self,
        predicate: impl Fn(&str) -> bool + 's,
    ) -> impl Iterator<Item = &'s SoundAlias> {
        self.aliases
            .iter()
            .filter(move |a| predicate(a.name.as_str()))
    }

    /// The payload an alias names, when it is a loaded sound.
    pub fn payload(&self, alias: &SoundAlias) -> Option<&LoadedSound> {
        let wanted = alias.loaded.as_deref()?;
        self.loaded.iter().find(|s| s.name == wanted)
    }

    /// The names of every alias, sorted: a quick look at what a zone holds.
    pub fn alias_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.aliases.iter().map(|a| a.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }
}

impl std::fmt::Debug for ZoneData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZoneData")
            .field("aliases", &self.aliases.len())
            .field("loaded", &self.loaded.len())
            .field("tables", &self.tables.len())
            .field("raw_files", &self.raw_files.len())
            .field("walked", &self.walked)
            .field("stopped_at", &self.stopped_at)
            .finish()
    }
}

/// Walk `image` and capture its sounds.
pub fn read_zone(image: &ZoneImage) -> std::result::Result<ZoneData, Error> {
    let mut memory = image.memory();
    let mut stream = memory.stream(image.bytes())?;
    let mut sink = Sink::default();

    stream.push(XFILE_BLOCK_VIRTUAL)?;
    let table = open_asset_table(&mut stream)?;
    for i in 0..table.count() {
        let ty = table.kind(&stream, i)?;
        let Some(slot) = table.slot(i) else {
            sink.stopped_at = Some(format!("asset {i} has no slot"));
            break;
        };
        if let Err(error) = load_asset_at_observed(&mut stream, ty, slot, &mut sink) {
            sink.stopped_at = Some(format!("asset {i} ({}): {error:?}", type_name(ty)));
            break;
        }
        sink.walked += 1;
    }
    stream.pop()?;

    Ok(ZoneData {
        aliases: sink.aliases,
        loaded: sink.loaded,
        tables: sink.tables,
        raw_files: sink.raw_files,
        walked: sink.walked,
        stopped_at: sink.stopped_at,
    })
}

fn type_name(ty: AssetType) -> &'static str {
    ty.name()
}

/// The capture half of the walk: everything the vendored reader reports about a sound.
#[derive(Default)]
struct Sink {
    aliases: Vec<SoundAlias>,
    loaded: Vec<LoadedSound>,
    tables: Vec<Table>,
    raw_files: Vec<RawFile>,
    /// The payload each sound file resolves to, keyed by where the file struct sits.
    file_to_payload: HashMap<(u8, u32), String>,
    /// The streamed file each sound file resolves to, keyed the same way.
    file_to_streamed: HashMap<(u8, u32), (String, String)>,
    /// The payload loaded by the most recent `LoadedSound`, which the next sound file binds to.
    last_loaded: Option<String>,
    walked: usize,
    stopped_at: Option<String>,
}

impl AssetLinkSink for Sink {
    fn loaded(
        &mut self,
        _s: &ZoneStream<'_>,
        _ty: AssetType,
        _slot: Ptr,
        _insert_slot: Option<Ptr>,
    ) -> fastfile_iw4::Result<()> {
        Ok(())
    }

    fn alias(&mut self, _ty: AssetType, _slot: Ptr, _target: Ptr) -> fastfile_iw4::Result<()> {
        Ok(())
    }

    fn capture_loaded_sound(
        &mut self,
        s: &ZoneStream<'_>,
        header: Ptr,
        pcm: Ptr,
        data_len: usize,
    ) -> fastfile_iw4::Result<()> {
        let name = name_at(s, header, 0).unwrap_or_default();
        let payload = s.slice_at(pcm, 0, data_len)?.to_vec();
        if name.is_empty() || payload.is_empty() {
            self.last_loaded = None;
            return Ok(());
        }
        // The 64-bit layout packs these as 16-bit fields; the 32-bit one uses 32-bit fields.
        let (format, rate, bits, channels, block_size) =
            if s.wire_format() == fastfile_iw4::Iw4WireFormat::X64 {
                (
                    i32::from(s.u16_at(header, 8)?),
                    s.u32_at(header, 12)?,
                    i32::from(s.u16_at(header, 22)?),
                    i32::from(s.u16_at(header, 10)?),
                    u32::from(s.u16_at(header, 20)?),
                )
            } else {
                (
                    s.i32_at(header, 4)?,
                    s.u32_at(header, 16)?,
                    s.i32_at(header, 20)?,
                    s.i32_at(header, 24)?,
                    s.u32_at(header, 32)?,
                )
            };
        self.last_loaded = Some(name.clone());
        self.loaded.push(LoadedSound {
            name,
            format,
            rate,
            bits,
            channels,
            block_size,
            bytes: payload,
        });
        Ok(())
    }

    fn bind_last_loaded_to_sound_file(&mut self, file: Ptr) -> fastfile_iw4::Result<()> {
        if let Some(name) = self.last_loaded.clone() {
            self.file_to_payload.insert(file_key(file), name);
        }
        Ok(())
    }

    fn bind_streamed_sound_file(
        &mut self,
        file: Ptr,
        dir: &str,
        name: &str,
    ) -> fastfile_iw4::Result<()> {
        self.file_to_streamed
            .insert(file_key(file), (dir.to_owned(), name.to_owned()));
        Ok(())
    }

    fn capture_string_table(
        &mut self,
        s: &ZoneStream<'_>,
        header: Ptr,
    ) -> fastfile_iw4::Result<()> {
        let name = name_at(s, header, 0).unwrap_or_default();
        let columns = s.i32_at(header, s.layout(4, 8)).unwrap_or(0).max(0) as usize;
        let rows = s.i32_at(header, s.layout(8, 12)).unwrap_or(0).max(0) as usize;
        let mut cells = Vec::with_capacity(columns.saturating_mul(rows));
        if let Ok(ZonePtr::Offset(target)) = s.ptr_at(header, s.layout(12, 16)) {
            let array = s.resolve_alias(target);
            for i in 0..columns.saturating_mul(rows) {
                let cell = array.at(i * s.layout(sz::STRING_TABLE_CELL, 16));
                cells.push(name_at(s, cell, 0).unwrap_or_default());
            }
        }
        if !name.is_empty() {
            self.tables.push(Table {
                name,
                columns,
                rows,
                cells,
            });
        }
        Ok(())
    }

    fn capture_raw_file(
        &mut self,
        name: &str,
        data: &[u8],
        zlib_compressed: bool,
    ) -> fastfile_iw4::Result<()> {
        if !name.is_empty() && !data.is_empty() {
            self.raw_files.push(RawFile {
                name: name.to_owned(),
                bytes: data.to_vec(),
                zlib_compressed,
            });
        }
        Ok(())
    }

    fn capture_sound(
        &mut self,
        s: &ZoneStream<'_>,
        list: Ptr,
        count: usize,
        head: Option<Ptr>,
    ) -> fastfile_iw4::Result<()> {
        let list_name = name_at(s, list, 0).unwrap_or_default();
        let Some(array) = head else {
            return Ok(());
        };
        for i in 0..count {
            let row = array.at(i * s.layout(sz::SND_ALIAS, 136));
            let name = name_at(s, row, s.layout(SND_ALIAS_ALIAS_NAME, 0)).unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let file = match s.ptr_at(row, s.layout(SND_ALIAS_SOUND_FILE, 40)) {
                Ok(ZonePtr::Offset(target)) => Some(s.resolve_alias(target)),
                _ => None,
            };
            let key = file.map(file_key);
            let loaded = key.and_then(|k| self.file_to_payload.get(&k).cloned());
            let streamed = key.and_then(|k| self.file_to_streamed.get(&k).cloned());
            let flags = s.u32_at(row, s.layout(SND_ALIAS_FLAGS, 80)).unwrap_or(0);
            self.aliases.push(SoundAlias {
                name,
                list: list_name.clone(),
                loaded: loaded.or_else(|| streamed.as_ref().map(|(_, name)| name.clone())),
                streamed,
                looping: flags & SND_ALIAS_FLAG_LOOPING != 0,
                volume: [
                    s.f32_at(row, s.layout(SND_ALIAS_VOL_MIN, 52))
                        .unwrap_or(0.0),
                    s.f32_at(row, s.layout(SND_ALIAS_VOL_MAX, 56))
                        .unwrap_or(0.0),
                ],
                pitch: [
                    s.f32_at(row, s.layout(SND_ALIAS_PITCH_MIN, 60))
                        .unwrap_or(0.0),
                    s.f32_at(row, s.layout(SND_ALIAS_PITCH_MAX, 64))
                        .unwrap_or(0.0),
                ],
                distance: [
                    s.f32_at(row, s.layout(SND_ALIAS_DIST_MIN, 68))
                        .unwrap_or(0.0),
                    s.f32_at(row, s.layout(SND_ALIAS_DIST_MAX, 72))
                        .unwrap_or(0.0),
                ],
                probability: s
                    .f32_at(row, s.layout(SND_ALIAS_PROBABILITY, 88))
                    .unwrap_or(1.0),
                sequence: s.i32_at(row, s.layout(SND_ALIAS_SEQUENCE, 48)).unwrap_or(0),
            });
        }
        Ok(())
    }
}

/// A zone pointer's identity, which is how a sound file is matched to its payload.
fn file_key(p: Ptr) -> (u8, u32) {
    (p.block, p.offset)
}

/// The string a struct's pointer field names.
fn name_at(s: &ZoneStream<'_>, parent: Ptr, field: usize) -> Option<String> {
    match s.ptr_at(parent, field).ok()? {
        ZonePtr::Offset(p) => s.cstr(s.resolve_alias(p)).ok().map(str::to_owned),
        ZonePtr::Null => Some(String::new()),
        _ => None,
    }
}
