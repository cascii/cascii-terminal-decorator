use anyhow::{Context, Result, bail, ensure};
use cascii_core_view::multisize::FrameBytes;
use reqwest::{Url, blocking::{Client, Response}, header::{ACCEPT_ENCODING, CONTENT_RANGE, RANGE}};
use std::{borrow::Cow, cell::RefCell, fs::{self, File}, io::{Read, Seek, SeekFrom}, ops::Range, path::{Path, PathBuf}, time::Duration};

#[derive(Clone)]
pub enum Input {
    Path(PathBuf),
    Url(Client, Url),
}

impl Input {
    pub fn from_path(path: &Path) -> Result<Self> {
        if let Some(text) = path.to_str().filter(|s| s.starts_with("https://") || s.starts_with("http://")) {
            let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
            return Ok(Self::Url(client, Url::parse(text)?));
        }
        Ok(Self::Path(path.to_owned()))
    }

    pub fn sibling(&self, name: &str) -> Result<Self> {
        match self {
            Self::Path(path) => Ok(Self::Path(path.with_file_name(name))),
            Self::Url(client, url) => Ok(Self::Url(client.clone(), url.join(name)?)),
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Path(path) => path.file_name()?.to_str(),
            Self::Url(_, url) => url.path_segments()?.next_back(),
        }
    }

    pub fn read_limited(&self, limit: usize) -> Result<Vec<u8>> {
        match self {
            Self::Path(path) => read_limited(File::open(path).with_context(|| format!("opening {}", path.display()))?, limit),
            Self::Url(client, url) => read_limited(client.get(url.clone()).send()?.error_for_status()?, limit),
        }
    }

    pub fn copy_to(&self, destination: &Path) -> Result<()> {
        match self {
            Self::Path(path) => {
                ensure!(fs::symlink_metadata(path)?.is_file(), "Expected a regular file at {}", path.display());
                fs::copy(path, destination)?;
            }
            Self::Url(client, url) => {
                let mut response = client.get(url.clone()).header(ACCEPT_ENCODING, "identity").send()?.error_for_status()?;
                response.copy_to(&mut File::create_new(destination)?)?;
            }
        }
        Ok(())
    }
}

pub fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut data)?;
    ensure!(data.len() <= limit, "Asset exceeds its declared size or format limit");
    Ok(data)
}

enum Storage {
    File(RefCell<File>),
    Http(Client, Url),
    Memory(Vec<u8>),
}

pub struct Bytes {
    storage: Storage,
    pub len: usize,
}

fn content_range(response: &Response, start: usize, end: usize) -> Result<usize> {
    ensure!(response.status() == reqwest::StatusCode::PARTIAL_CONTENT, "Server did not return a byte range");
    let value = response.headers().get(CONTENT_RANGE).context("Missing Content-Range")?.to_str()?;
    let total = value.strip_prefix(&format!("bytes {start}-{end}/")).context("Incorrect Content-Range")?.parse()?;
    ensure!(end < total, "Invalid total size in Content-Range");
    Ok(total)
}

impl Bytes {
    pub fn open(input: &Input) -> Result<Self> {
        let (storage, len) = match input {
            Input::Path(path) => {
                let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
                let len = usize::try_from(file.metadata()?.len())?;
                (Storage::File(RefCell::new(file)), len)
            }
            Input::Url(client, url) => {
                let response = client.get(url.clone()).header(ACCEPT_ENCODING, "identity").header(RANGE, "bytes=0-0").send()?.error_for_status()?;
                if response.status() == reqwest::StatusCode::OK {
                    const LIMIT: usize = 16 * 1024 * 1024;
                    ensure!(response.content_length().is_none_or(|n| n <= LIMIT as u64), "Large remote files require HTTP byte-range support");
                    let data = read_limited(response, LIMIT)?;
                    let len = data.len();
                    (Storage::Memory(data), len)
                } else {
                    let total = content_range(&response, 0, 0)?;
                    ensure!(read_limited(response, 1)?.len() == 1, "Truncated HTTP range");
                    (Storage::Http(client.clone(), url.clone()), total)
                }
            }
        };
        Ok(Self {storage, len})
    }

    pub fn read(&self, range: Range<usize>) -> Result<Vec<u8>> {
        ensure!(range.start <= range.end && range.end <= self.len, "Truncated input");
        if range.is_empty() { return Ok(Vec::new()); }
        match &self.storage {
            Storage::File(file) => {
                let mut file = file.borrow_mut();
                file.seek(SeekFrom::Start(range.start as u64))?;
                let mut bytes = vec![0; range.len()];
                file.read_exact(&mut bytes)?;
                Ok(bytes)
            }
            Storage::Memory(bytes) => Ok(bytes[range].to_vec()),
            Storage::Http(client, url) => {
                let end = range.end - 1;
                let response = client.get(url.clone()).header(ACCEPT_ENCODING, "identity").header(RANGE, format!("bytes={}-{end}", range.start)).send()?.error_for_status()?;
                ensure!(content_range(&response, range.start, end)? == self.len, "Remote asset size changed");
                let data = read_limited(response, range.len())?;
                if data.len() != range.len() { bail!("Truncated HTTP range"); }
                Ok(data)
            }
        }
    }
}

impl FrameBytes for Bytes {
    fn read_range(&self, range: Range<usize>) -> Result<Cow<'_, [u8]>, String> {
        self.read(range).map(Cow::Owned).map_err(|e| format!("{e:#}"))
    }
}
