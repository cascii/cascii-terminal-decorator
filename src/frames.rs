use crate::{collect_frame_paths, find_standalone_binary, input::{Bytes, Input}, normalize_frame_text, tiles::{self, Tiles}};
use anyhow::{Context, Result, anyhow, bail, ensure};
use cascii_core_view::{CFRAME_EXT_FLAG_HAS_BG, Frame, MultiSizeManifest, multisize::IndexedMultiSizeAsset, parse_cframe};
use std::{fs, ops::Range, path::{Path, PathBuf}};

pub enum Frames {
    Loose(Vec<(Option<PathBuf>, Option<PathBuf>)>),
    FullPack(Bytes, Vec<Range<usize>>),
    Legacy {bytes: Bytes, count: usize, dimensions: [u8; 8], stride: usize, flag: Option<usize>},
    Binary(IndexedMultiSizeAsset<Bytes>),
    Compressed(IndexedMultiSizeAsset<Vec<u8>>),
    Tiles(Box<Tiles>),
}

fn word(bytes: &[u8], pos: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(bytes.get(pos..pos + 4).context("Truncated frame header")?.try_into()?) as usize)
}

fn decode(bytes: &[u8]) -> Result<Frame> {
    let width = word(bytes, 0)?;
    let height = word(bytes, 4)?;
    let cells = width.checked_mul(height).context("Frame dimensions overflow")?;
    ensure!(cells > 0 && cells <= bytes.len().saturating_sub(8) / 4, "Truncated or empty CFrame");
    let frame = parse_cframe(bytes)?;
    Ok(Frame::with_color(frame.to_text(), frame))
}

impl Frames {
    pub fn open(input: &Input) -> Result<Self> {
        if let Input::Path(path) = input {
            if path.is_dir() {
                if let Some(binary) = find_standalone_binary(path)? {
                    return Self::open(&Input::Path(binary));
                }
                return Self::directory(path);
            }
            if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("cframe")) {
                return Ok(Self::Loose(vec![(None, Some(path.clone()))]));
            }
            if path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("txt")) {
                return Ok(Self::Loose(vec![(Some(path.clone()), None)]));
            }
        }
        let bytes = Bytes::open(input)?;
        let header = bytes.read(0..20.min(bytes.len))?;
        if header.starts_with(b"CSMV") {
            let len = bytes.len;
            return IndexedMultiSizeAsset::open(bytes, len).map(Self::Binary).map_err(|e| anyhow!("Invalid CSMV: {e}"));
        }
        if tiles::is_archive(&header) {
            return Ok(Self::Tiles(Box::new(Tiles::archive(bytes, &header)?)));
        }
        let compressed = header.starts_with(&[31, 139, 8]);
        let json = !header.contains(&0) && header.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{');
        if compressed || json || input.name().is_some_and(|s| s.ends_with(".json")) {
            let raw = tiles::manifest_or_tile(&bytes, compressed)?;
            if raw.starts_with(b"CSMV") {
                let len = raw.len();
                return IndexedMultiSizeAsset::open(raw, len).map(Self::Compressed).map_err(|e| anyhow!("Invalid compressed CSMV: {e}"));
            }
            return Ok(Self::Tiles(Box::new(Tiles::from_manifest(input, &raw)?)));
        }
        if header.starts_with(b"CFPK") {
            let count = word(&header, 8)?;
            ensure!(word(&header, 4)? == 1 && count > 0 && count <= bytes.len.saturating_sub(12) / 16, "Invalid CFPK version or frame count");
            let mut entries = Vec::new();
            let mut start = 12usize;
            for _ in 0..count {
                let len = word(&bytes.read(start..start + 4)?, 0)?;
                start += 4;
                let end = start.checked_add(len).context("CFPK frame offset overflow")?;
                ensure!(len >= 12 && end <= bytes.len, "Truncated CFPK frame");
                entries.push(start..end);
                start = end;
            }
            ensure!(start == bytes.len, "Trailing CFPK bytes");
            return Ok(Self::FullPack(bytes, entries));
        }
        let count = word(&header, 0)?;
        let cells = word(&header, 4)?.checked_mul(word(&header, 8)?).context("Invalid packed dimensions")?;
        let body = cells.checked_mul(4).context("Invalid packed dimensions")?;
        let available = bytes.len - 12;
        ensure!(count > 0 && cells > 0 && body <= available / count, "Unrecognized or truncated packed animation");
        let bg = cells * 3;
        let flagged = body.checked_add(bg).and_then(|n| n.checked_add(1));
        let exact_stride = (available % count == 0).then_some(available / count);
        let (stride, flag) = match exact_stride {
            Some(stride) if Some(stride) == flagged => (stride, Some(body)),
            Some(stride) if Some(stride) == body.checked_add(bg) => (stride, None),
            _ => (body, None),
        };
        Ok(Self::Legacy {bytes, count, dimensions: header[4..12].try_into()?, stride, flag})
    }

    fn directory(path: &Path) -> Result<Self> {
        let text = collect_frame_paths(path, "txt", true)?;
        let pairs = if text.is_empty() {
            collect_frame_paths(path, "cframe", false)?.into_iter().map(|path| (None, Some(path))).collect()
        } else {
            text.into_iter().map(|path| {
                let color = path.with_extension("cframe");
                (Some(path), color.is_file().then_some(color))
            }).collect::<Vec<_>>()
        };
        ensure!(!pairs.is_empty(), "No frame files found in {}", path.display());
        Ok(Self::Loose(pairs))
    }

    pub fn manifest(&self) -> Option<&MultiSizeManifest> {
        match self {
            Self::Binary(asset) => Some(&asset.manifest),
            Self::Compressed(asset) => Some(&asset.manifest),
            Self::Tiles(tiles) => Some(&tiles.manifest),
            _ => None,
        }
    }

    pub fn count(&self) -> usize {
        match self {
            Self::Loose(files) => files.len(),
            Self::FullPack(_, frames) => frames.len(),
            Self::Legacy {count, ..} => *count,
            Self::Binary(asset) => asset.manifest.frame_count as usize,
            Self::Compressed(asset) => asset.manifest.frame_count as usize,
            Self::Tiles(tiles) => tiles.manifest.frame_count as usize,
        }
    }

    pub fn has_color(&self) -> bool {
        match self {
            Self::Loose(files) => files.iter().any(|(_, color)| color.is_some()),
            _ => self.manifest().is_none_or(|manifest| manifest.color),
        }
    }

    pub fn frame(&mut self, index: usize, columns: u32, use_color: bool) -> Result<Frame> {
        ensure!(index < self.count(), "Frame index out of range");
        let mut frame = match self {
            Self::Loose(files) => {
                let (txt, color) = &files[index];
                let text = txt.as_ref().map(fs::read_to_string).transpose()?.map(normalize_frame_text);
                if let Some(path) = color.as_ref().filter(|_| use_color || text.is_none()) {
                    let mut frame = decode(&fs::read(path)?).with_context(|| format!("reading {}", path.display()))?;
                    if let Some(text) = text { frame.content = text; }
                    frame
                } else {
                    Frame::text_only(text.context("Missing frame text")?)
                }
            }
            Self::FullPack(bytes, ranges) => decode(&bytes.read(ranges[index].clone())?)?,
            Self::Legacy {bytes, dimensions, stride, flag, ..} => {
                let start = 12 + *stride * index;
                let raw = bytes.read(start..start + *stride)?;
                if flag.is_some_and(|pos| raw[pos] & CFRAME_EXT_FLAG_HAS_BG == 0) { bail!("Invalid packed background flag"); }
                let mut data = Vec::with_capacity(8 + raw.len());
                data.extend_from_slice(dimensions);
                data.extend_from_slice(&raw);
                decode(&data)?
            }
            Self::Binary(asset) => asset.frame(columns, index).map_err(|e| anyhow!(e))?,
            Self::Compressed(asset) => asset.frame(columns, index).map_err(|e| anyhow!(e))?,
            Self::Tiles(tiles) => tiles.frame(columns, index)?,
        };
        if !use_color { frame.cframe = None; }
        Ok(frame)
    }

    pub fn validate(&mut self) -> Result<()> {
        let widths = self.manifest().map(|m| m.variants.iter().map(|v| v.columns).collect()).unwrap_or_else(|| vec![0]);
        for width in widths {
            for frame in 0..self.count() {
                self.frame(frame, width, true).with_context(|| format!("validating frame {} at width {width}", frame + 1))?;
            }
        }
        Ok(())
    }
}
