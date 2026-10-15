//! Inflating a Modern Warfare 2 fastfile into the zone image the container reader walks.
//!
//! An `.ff` under `<MW2_DATA>/zone/…` is a 21-byte IW4 file header over one zlib stream that
//! inflates to the zone image: an XFile header (eight block sizes) followed by every asset body,
//! in the order the game loads them. `fastfile_iw4` reads that image; this module only gets to it.
//!
//! The two container rules transcribed here — the signed-file chunk layout and the wire-format
//! probe — follow IW4L's `asset_transport::zone` (Apache-2.0; see `vendor/iw4/README.md`).

use std::path::{Path, PathBuf};

use fastfile_iw4::{
    FILE_PREAMBLE_LEN, Iw4WireFormat, MAGIC_AUTH_HEADER, MAX_XFILE_COUNT, Signing,
    XFILE_HEADER_LEN, ZoneHeader, ZoneStream, parse_file_header, parse_zone_header,
    read_wire_asset_table,
};

/// A signed zone stores its payload in 8 KiB chunks, one header chunk per 256.
const AUTHED_CHUNK_SIZE: usize = 0x2000;
const AUTHED_CHUNKS_PER_GROUP: usize = 256;

/// Why a fastfile could not be read.
#[derive(Debug)]
pub enum Error {
    /// The file could not be opened or read.
    Io(std::io::Error),
    /// The 21-byte file header is not an IW4 fastfile header.
    Header(fastfile_iw4::FileHeaderError),
    /// The zlib stream after the header is not intact.
    Inflate(String),
    /// A signed file whose payload does not start with the authentication chunk header.
    NotAuthenticated,
    /// The inflated image is shorter than an XFile header.
    Truncated,
    /// The asset table validates as neither the 32- nor the 64-bit wire layout.
    NoWireLayout {
        x86: fastfile_iw4::WireTableError,
        x64: fastfile_iw4::WireTableError,
    },
    /// The zone could not be walked.
    Zone(fastfile_iw4::ZoneError),
    /// An `.iwd` archive entry could not be read.
    Archive(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "reading the fastfile: {e}"),
            Self::Header(e) => write!(f, "not an IW4 fastfile: {e:?}"),
            Self::Inflate(e) => write!(f, "inflating the fastfile: {e}"),
            Self::NotAuthenticated => write!(f, "signed fastfile without an auth header"),
            Self::Truncated => write!(f, "the inflated zone is shorter than its XFile header"),
            Self::NoWireLayout { x86, x64 } => write!(
                f,
                "the asset table validates as neither wire layout (x86: {x86:?}; x64: {x64:?})"
            ),
            Self::Zone(e) => write!(f, "{e:?}"),
            Self::Archive(e) => write!(f, "reading an .iwd entry: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<fastfile_iw4::ZoneError> for Error {
    fn from(e: fastfile_iw4::ZoneError) -> Self {
        Self::Zone(e)
    }
}

/// An inflated fastfile: the bytes the container walk reads, and the header describing them.
pub struct ZoneImage {
    bytes: Vec<u8>,
    header: ZoneHeader,
    format: Iw4WireFormat,
    path: PathBuf,
}

impl ZoneImage {
    /// The inflated zone image.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The XFile header: the image's size and its eight block sizes.
    pub fn header(&self) -> &ZoneHeader {
        &self.header
    }

    /// Whether this zone's pointers are 4 or 8 bytes wide.
    pub fn format(&self) -> Iw4WireFormat {
        self.format
    }

    /// The file this image came from, for a message.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The mutable block buffers a [`ZoneStream`] loads asset bodies into.
    ///
    /// The blocks are separate allocations, not views into the image: the walk copies each body
    /// out of the image (its stream) into the block its pointers name, which is what lets a
    /// pointer be resolved at all. IW4L's `asset_transport::zone::ZoneMemory` is the same rule.
    pub fn memory(&self) -> Memory {
        Memory::new(&self.header)
    }
}

/// The block buffers and insert map a zone walk allocates into.
pub struct Memory {
    blocks: [Vec<u8>; MAX_XFILE_COUNT],
    insert_map: Vec<u8>,
}

impl Memory {
    fn new(header: &ZoneHeader) -> Memory {
        Memory {
            blocks: std::array::from_fn(|i| vec![0u8; header.block_size[i] as usize]),
            insert_map: vec![0u8; ZoneStream::insert_map_len(header)],
        }
    }

    /// A stream over `image` that loads into these blocks.
    ///
    /// # Panics
    ///
    /// Never: the block sizes come from the same header the image was parsed with.
    pub fn stream<'a>(&'a mut self, image: &'a [u8]) -> fastfile_iw4::Result<ZoneStream<'a>> {
        let [b0, b1, b2, b3, b4, b5, b6, b7] = &mut self.blocks;
        let blocks: [&mut [u8]; MAX_XFILE_COUNT] = [
            b0.as_mut_slice(),
            b1.as_mut_slice(),
            b2.as_mut_slice(),
            b3.as_mut_slice(),
            b4.as_mut_slice(),
            b5.as_mut_slice(),
            b6.as_mut_slice(),
            b7.as_mut_slice(),
        ];
        ZoneStream::new(image, blocks, &mut self.insert_map)
    }
}

/// Inflate the fastfile at `path`.
pub fn open_zone(path: impl AsRef<Path>) -> Result<ZoneImage, Error> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    parse_zone_image(&bytes, path)
}

/// [`open_zone`] over bytes already in memory.
pub fn parse_zone_image(bytes: &[u8], path: &Path) -> Result<ZoneImage, Error> {
    let header = parse_file_header(bytes).map_err(Error::Header)?;
    let body = bytes.get(FILE_PREAMBLE_LEN..).ok_or(Error::Truncated)?;
    let owned;
    let compressed: &[u8] = match header.signing {
        Signing::Unsigned => body,
        Signing::Signed => {
            owned = dechunk_authed(body)?;
            &owned
        }
    };
    let image = inflate(compressed)?;
    if image.len() < XFILE_HEADER_LEN {
        return Err(Error::Truncated);
    }
    let zone = parse_zone_header(&image)?;
    let format = match (
        read_wire_asset_table(&image, Iw4WireFormat::X86),
        read_wire_asset_table(&image, Iw4WireFormat::X64),
    ) {
        (Ok(table), Err(_)) => table.format(),
        (Err(_), Ok(table)) => table.format(),
        (Err(x86), Err(x64)) => return Err(Error::NoWireLayout { x86, x64 }),
        (Ok(_), Ok(_)) => Iw4WireFormat::X64,
    };
    Ok(ZoneImage {
        bytes: image,
        header: zone,
        format,
        path: path.to_path_buf(),
    })
}

/// A signed fastfile's payload: the first chunk is the authentication header, then one chunk in
/// every group of 257 is a signature rather than data.
fn dechunk_authed(body: &[u8]) -> Result<Vec<u8>, Error> {
    if body.len() < 8 || &body[0..8] != MAGIC_AUTH_HEADER {
        return Err(Error::NotAuthenticated);
    }
    let mut out = Vec::with_capacity(body.len());
    let mut pos = AUTHED_CHUNK_SIZE;
    let mut chunk_in_group = 0usize;
    while pos < body.len() {
        let end = (pos + AUTHED_CHUNK_SIZE).min(body.len());
        if chunk_in_group != 0 {
            out.extend_from_slice(&body[pos..end]);
        }
        pos = end;
        chunk_in_group = (chunk_in_group + 1) % (AUTHED_CHUNKS_PER_GROUP + 1);
    }
    Ok(out)
}

/// Inflate one zlib stream, for a raw file the zone stored compressed.
pub(crate) fn inflate_zlib(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    inflate(bytes)
}

/// Inflate one zlib stream, chunk by chunk so a 200 MB zone does not need a guessed capacity.
fn inflate(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    use flate2::{Decompress, FlushDecompress, Status};
    let mut decoder = Decompress::new(true);
    let mut out = Vec::with_capacity(bytes.len());
    loop {
        out.reserve((out.len() / 2).max(1024 * 1024));
        let before = (decoder.total_in(), decoder.total_out());
        let status = decoder
            .decompress_vec(
                &bytes[decoder.total_in() as usize..],
                &mut out,
                FlushDecompress::None,
            )
            .map_err(|e| Error::Inflate(e.to_string()))?;
        if status == Status::StreamEnd {
            return Ok(out);
        }
        if before == (decoder.total_in(), decoder.total_out()) {
            return Err(Error::Inflate("truncated zlib stream".into()));
        }
    }
}
