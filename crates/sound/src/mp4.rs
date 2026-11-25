use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use jiff::SignedDuration;
use jiff::fmt::rfc2822::DateTimeParser;
use reqwest::header::{AUTHORIZATION, CONTENT_RANGE, DATE, ETAG, IF_RANGE, LAST_MODIFIED, RANGE};
use symphonia::core::io::MediaSourceStreamOptions;

use crate::ranged::{Address, File, Reach};
use crate::stream::{Plain, Reader, Stream};

/// The largest metadata suffix retained separately from the sequential download.
const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
const RANGE_PATIENCE: Duration = Duration::from_secs(5);
/// Allows for second-resolution modification times and clock skew between HTTP dates.
const VALIDATOR_AGE: SignedDuration = SignedDuration::from_secs(60);

/// A validated suffix in the original file's byte coordinates.
pub struct Tail {
    start: u64,
    bytes: Vec<u8>,
}

/// How to open a track without making the audio thread wait for its metadata.
#[derive(Clone, Default)]
pub struct Prepared {
    progressive: bool,
    tail: Option<Arc<Tail>>,
}

/// The original stream response's address and validator, reused for metadata ranges.
pub struct Remote {
    url: reqwest::Url,
    /// The `Authorization` header the original request carried, for a server whose urls hold no
    /// credentials of their own.
    authorization: Option<reqwest::header::HeaderValue>,
    validator: Option<reqwest::header::HeaderValue>,
    total: u64,
}

impl Remote {
    pub fn new(response: &reqwest::Response) -> Option<Self> {
        Some(Self {
            url: response.url().clone(),
            authorization: None,
            validator: range_validator(response.headers()),
            total: response.content_length()?,
        })
    }

    /// Sends `value` as the `Authorization` header of every range request, as long as the file
    /// is still served from `server`. A file a redirect moved to another origin gets no
    /// credentials, as the redirect itself sent none.
    pub fn authorized(mut self, server: &str, value: &str) -> Self {
        let home =
            reqwest::Url::parse(server).is_ok_and(|server| server.origin() == self.url.origin());
        self.authorization = home
            .then(|| reqwest::header::HeaderValue::from_str(value).ok())
            .flatten();
        self
    }

    /// What a seek past `main`, the download of track `id`, fetches the file through, with the
    /// credentials of the range requests. `None` for a file no seek can enter part way.
    pub fn file(
        self,
        name: &'static str,
        id: &str,
        http: reqwest::Client,
        main: &Stream,
    ) -> Option<Arc<File>> {
        let reach = Reach::new(name, id, move |_| {
            let authorization = self
                .authorization
                .iter()
                .map(|value| (AUTHORIZATION, value.clone()));
            let address = Address::new(name, http.clone(), self.url.to_string());
            (address.headers(authorization.collect()), Plain)
        });
        File::detect(main, reach)
    }

    /// Accepts only an exact partial response and bounds both its size and its lifetime.
    async fn tail(&self, http: &reqwest::Client, start: u64) -> Option<Tail> {
        let validator = self.validator.as_ref()?;
        let length = self.total.checked_sub(start)?;
        if length == 0 || length > METADATA_LIMIT {
            return None;
        }
        let mut request = http
            .get(self.url.clone())
            .header(RANGE, format!("bytes={start}-{}", self.total - 1))
            .header(IF_RANGE, validator);
        if let Some(authorization) = &self.authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        tokio::time::timeout(RANGE_PATIENCE, async {
            let mut response = request.send().await.ok()?;
            let expected = format!("bytes {start}-{}/{}", self.total - 1, self.total);
            if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                || response.headers().get(CONTENT_RANGE)?.to_str().ok()? != expected
                || response.content_length().is_some_and(|size| size != length)
            {
                return None;
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.ok()? {
                if bytes.len() as u64 + chunk.len() as u64 > length {
                    return None;
                }
                bytes.extend_from_slice(&chunk);
            }
            (bytes.len() as u64 == length).then_some(Tail { start, bytes })
        })
        .await
        .ok()
        .flatten()
    }
}

impl Prepared {
    /// Waits for MP4 metadata on the cancellable load task. A server that refuses ranges falls
    /// back to the original sequential download without holding the audio thread. An Ogg file
    /// stays unseekable until it has all arrived, because Symphonia reads its last pages on open.
    pub async fn load(
        stream: &mut Stream,
        remote: Option<(&reqwest::Client, &Remote)>,
    ) -> Result<Self> {
        let mut at = 0usize;
        loop {
            let wanted = at
                .checked_add(16)
                .ok_or_else(|| anyhow::anyhow!("mp4 atom offset overflows"))?;
            stream.wait_for(wanted).await?;
            let header = stream
                .with(|_, spool| {
                    (spool.len() >= wanted)
                        .then(|| spool.bytes(at..wanted))
                        .transpose()
                })
                .transpose()?
                .flatten();
            let Some(header) = header else {
                return Ok(Self::default());
            };
            if at == 0 && header.starts_with(b"OggS") {
                return Ok(Self {
                    progressive: !stream.done(),
                    tail: None,
                });
            }
            let kind = &header[4..8];
            if at == 0 && kind != b"ftyp" {
                return Ok(Self::default());
            }
            let length = match u32::from_be_bytes(header[..4].try_into()?) {
                1 => u64::from_be_bytes(header[8..16].try_into()?),
                0 => {
                    stream.wait_for(usize::MAX).await?;
                    return Ok(Self::default());
                }
                length if length >= 8 => u64::from(length),
                _ => bail!("mp4 atom is shorter than its header"),
            };
            if length < 8 || (header[..4] == 1u32.to_be_bytes() && length < 16) {
                bail!("mp4 atom is shorter than its header");
            }
            let end = usize::try_from(length)
                .ok()
                .and_then(|length| at.checked_add(length))
                .ok_or_else(|| anyhow::anyhow!("mp4 atom offset overflows"))?;
            if kind == b"mdat" || kind == b"moof" {
                if !stream.done()
                    && kind == b"mdat"
                    && let Some((http, remote)) = remote
                {
                    // Symphonia consumes one ring buffer before a skipped atom's end.
                    let overlap = MediaSourceStreamOptions::default().buffer_len as u64;
                    let start = (end as u64).saturating_sub(overlap);
                    if let Some(tail) = remote.tail(http, start).await {
                        return Ok(Self {
                            progressive: false,
                            tail: Some(Arc::new(tail)),
                        });
                    }
                }
                stream.wait_for(usize::MAX).await?;
                return Ok(Self::default());
            }
            if kind == b"moov" {
                stream.wait_for(end).await?;
                return Ok(Self {
                    progressive: !stream.done(),
                    tail: None,
                });
            }
            at = end;
        }
    }

    /// Whether a decoder may seek, given the `total` the caller already passed to it as the
    /// byte length. Symphonia panics seeking an Ogg file it was told is seekable with no length.
    pub fn seekable(&self, total: Option<u64>, stream: &Stream) -> bool {
        total.is_some() && (!self.progressive || stream.done())
    }

    pub fn reader(&self, stream: &Stream) -> Input {
        Input {
            reader: stream.reader(),
            tail: self.tail.clone(),
            total: stream.total(),
            at: 0,
        }
    }
}

/// Reads fetched metadata at its original offsets while media keeps arriving sequentially.
/// The vault always receives the unmodified original file.
pub struct Input {
    reader: Reader,
    tail: Option<Arc<Tail>>,
    total: Option<u64>,
    at: u64,
}

impl Input {
    /// Reads `reader` as it is, with no fetched tail and no known end, for a reader spliced
    /// part way into a file that a decoder only reads forward.
    pub fn spliced(reader: Reader) -> Self {
        Self {
            reader,
            tail: None,
            total: None,
            at: 0,
        }
    }
}

impl Read for Input {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.total.is_some_and(|total| self.at >= total) {
            return Ok(0);
        }
        let mut wanted = out.len();
        if let Some(tail) = &self.tail {
            if self.at >= tail.start {
                let offset = usize::try_from(self.at - tail.start).map_err(io::Error::other)?;
                let available = tail.bytes.get(offset..).unwrap_or_default();
                let count = wanted.min(available.len());
                out[..count].copy_from_slice(&available[..count]);
                self.at += count as u64;
                return Ok(count);
            }
            wanted = wanted.min(usize::try_from(tail.start - self.at).unwrap_or(usize::MAX));
        }
        self.reader.seek(SeekFrom::Start(self.at))?;
        let count = self.reader.read(&mut out[..wanted])?;
        self.at += count as u64;
        Ok(count)
    }
}

impl Seek for Input {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.reader.seek(SeekFrom::Start(self.at))?;
        self.at = self.reader.seek(pos)?;
        Ok(self.at)
    }
}

/// Selects a strong If-Range validator. A date is usable only without an ETag and with
/// enough separation from Date to account for timestamp resolution and clock skew.
fn range_validator(headers: &reqwest::header::HeaderMap) -> Option<reqwest::header::HeaderValue> {
    if let Some(etag) = headers.get(ETAG) {
        let bytes = etag.as_bytes();
        let strong = bytes.len() >= 2
            && bytes.first() == Some(&b'"')
            && bytes.last() == Some(&b'"')
            && bytes[1..bytes.len() - 1]
                .iter()
                .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte) || *byte >= 0x80);
        return strong.then(|| etag.clone());
    }
    let modified = headers.get(LAST_MODIFIED)?;
    let dates = DateTimeParser::new();
    let modified_at = dates.parse_timestamp(modified.as_bytes()).ok()?;
    let dated_at = dates.parse_timestamp(headers.get(DATE)?.as_bytes()).ok()?;
    (dated_at.duration_since(modified_at) >= VALIDATOR_AGE).then(|| modified.clone())
}
