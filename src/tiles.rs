use crate::input::{Bytes, Input, read_limited};
use anyhow::{Context, Result, anyhow, ensure};
use cascii_core_view::{Frame, MultiSizeManifest, multisize::IndexedMultiSizeAsset};
use flate2::bufread::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet, VecDeque}, path::Path};

const MAX_TILE: usize = 16 * 1024 * 1024;
const MAX_INDEX: usize = 16 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Transport {
    format: String,
    version: u32,
    frames_per_tile: usize,
    manifest: MultiSizeManifest,
    archive: Option<Archive>,
    tiles: Vec<Tile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Archive {
    format: String,
    version: u32,
    payload_bytes: usize,
}

#[derive(Deserialize)]
struct Tile {
    url: String,
    columns: u32,
    start: usize,
    count: usize,
    bytes: usize,
    sha256: String,
    offset: Option<usize>,
    length: Option<usize>,
}

enum TileInput {
    Archive(Bytes, usize),
    Folder(Input),
}

type CachedTile = ((u32, usize), IndexedMultiSizeAsset<Vec<u8>>);

pub struct Tiles {
    pub manifest: MultiSizeManifest,
    input: TileInput,
    span: usize,
    tiles: HashMap<(u32, usize), Tile>,
    cache: VecDeque<CachedTile>,
}

fn inflate(compressed: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(compressed);
    let bytes = read_limited(&mut decoder, limit)?;
    ensure!(decoder.into_inner().is_empty(), "Unexpected bytes after gzip member");
    Ok(bytes)
}

pub fn is_archive(header: &[u8]) -> bool {
    header.len() >= 20 && header[..4] == [31, 139, 8, 4] && header[10..16] == [8, 0, b'C', b'A', 4, 0]
}

pub fn manifest_or_tile(bytes: &Bytes, compressed: bool) -> Result<Vec<u8>> {
    ensure!(bytes.len <= MAX_INDEX, "Tile or manifest exceeds size limit");
    let data = bytes.read(0..bytes.len)?;
    if compressed { inflate(&data, MAX_INDEX) } else { Ok(data) }
}

impl Tiles {
    pub fn archive(bytes: Bytes, header: &[u8]) -> Result<Self> {
        let index_size = u32::from_le_bytes(header[16..20].try_into()?) as usize;
        ensure!((28..=1024 * 1024).contains(&index_size), "Invalid archive index size");
        let json = inflate(&bytes.read(0..index_size)?, MAX_INDEX)?;
        Self::parse(TileInput::Archive(bytes, index_size), &json)
    }

    pub fn from_manifest(input: &Input, json: &[u8]) -> Result<Self> {
        Self::parse(TileInput::Folder(input.clone()), json)
    }

    fn parse(tile_input: TileInput, json: &[u8]) -> Result<Self> {
        let transport: Transport = serde_json::from_slice(json).context("reading tiled animation manifest")?;
        ensure!(transport.format == "cascii-tiled-animation" && transport.version == 1, "Unsupported tile manifest");
        transport.manifest.validate().map_err(|e| anyhow!(e))?;
        let span = transport.frames_per_tile;
        let frames = transport.manifest.frame_count as usize;
        ensure!((1..=64).contains(&span), "Invalid tile span");
        ensure!(transport.tiles.len() == frames.div_ceil(span) * transport.manifest.variants.len(), "Incomplete tile index");
        let mut tiles = HashMap::new();
        let mut names = HashSet::new();
        let mut offset = 0;
        for tile in transport.tiles {
            let name = tile.url.strip_suffix(".bin.gz").context("Invalid tile filename")?;
            ensure!(!name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') && names.insert(tile.url.clone()), "Invalid or duplicate tile filename");
            ensure!(transport.manifest.variants.iter().any(|v| v.columns == tile.columns) && tile.start < frames && tile.start % span == 0 && tile.count == span.min(frames - tile.start), "Invalid tile dimensions or time span");
            ensure!((16..=MAX_TILE).contains(&tile.bytes) && tile.sha256.len() == 64 && tile.sha256.bytes().all(|b| b.is_ascii_hexdigit()), "Invalid tile size or digest");
            if matches!(tile_input, TileInput::Archive(..)) {
                let length = tile.length.context("Missing tile length")?;
                ensure!(tile.offset == Some(offset) && (1..=MAX_TILE).contains(&length), "Invalid archive tile range");
                offset = offset.checked_add(length).context("Archive length overflow")?;
            }
            ensure!(tiles.insert((tile.columns, tile.start), tile).is_none(), "Duplicate tile range");
        }
        match (&tile_input, transport.archive) {
            (TileInput::Archive(bytes, index_size), Some(a)) => ensure!(a.format == "cascii-preload-archive" && a.version == 1 && a.payload_bytes == offset && bytes.len - index_size == offset, "Archive payload size or version mismatch"),
            (TileInput::Folder(_), None) => {}
            _ => anyhow::bail!("Archive metadata does not match its container"),
        }
        Ok(Self {manifest: transport.manifest, input: tile_input, span, tiles, cache: VecDeque::new()})
    }

    pub fn copy_sidecars_from(&self, base: &Input, directory: &Path) -> Result<()> {
        if matches!(self.input, TileInput::Folder(_)) {
            for tile in self.tiles.values() {
                base.sibling(&tile.url)?.copy_to(&directory.join(&tile.url))?;
            }
        }
        Ok(())
    }

    pub fn frame(&mut self, columns: u32, frame: usize) -> Result<Frame> {
        let start = frame / self.span * self.span;
        let key = (columns, start);
        let asset = if let Some(i) = self.cache.iter().position(|(k, _)| *k == key) {
            self.cache.remove(i).unwrap().1
        } else {
            let tile = self.tiles.get(&key).context("Missing animation tile")?;
            let encoded = match &self.input {
                TileInput::Archive(bytes, index_size) => {
                    let pos = index_size + tile.offset.unwrap();
                    bytes.read(pos..pos + tile.length.unwrap())?
                }
                TileInput::Folder(base) => base.sibling(&tile.url)?.read_limited(MAX_TILE)?,
            };
            let raw = if encoded.starts_with(&[31, 139]) {inflate(&encoded, tile.bytes)?} else {encoded};
            ensure!(raw.len() == tile.bytes && format!("{:x}", Sha256::digest(&raw)).eq_ignore_ascii_case(&tile.sha256), "Tile {} failed size or SHA-256 verification", tile.url);
            let len = raw.len();
            let asset = IndexedMultiSizeAsset::open(raw, len).map_err(|e| anyhow!(e))?;
            let outer = self.manifest.variants.iter().find(|v| v.columns == columns).unwrap();
            let variants = &asset.manifest.variants;
            ensure!(asset.manifest.frame_count as usize == tile.count && variants.len() == 1 && variants[0].columns == columns && variants[0].rows == outer.rows[start..start + tile.count] && asset.manifest.color == self.manifest.color, "Tile disagrees with animation manifest");
            asset
        };
        self.cache.push_back((key, asset));
        // Four tiles, each limited to 16 MiB, bound the inflated data retained while seeking.
        if self.cache.len() > 4 { self.cache.pop_front(); }
        self.cache.back().unwrap().1.frame(columns, frame - start).map_err(|e| anyhow!(e))
    }
}
