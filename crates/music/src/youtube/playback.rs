//! Playback for YouTube Music: what [`crate::engine`] needs that is YouTube's own.
//!
//! The track streams through [`crate::stream`] like every other provider's, fed by ytmusic's
//! ranged download, which asks again from the byte a stalled connection stopped at. The file is
//! DASH fMP4, so the decoder opens it unseekable and a seek splices the header onto the segment
//! the `sidx` names, the way Apple Music's does. The threads, the queue, the preload, the gapless
//! join and the loudness gain are the engine's.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use bytes::Bytes;
use ytmusic::{AudioStream, YtMusic};

use crate::audio::Trimmed;
use crate::engine::{self, Fetch, Loudness};
use crate::route::{Codec, Encoding};
use crate::stream::{PREROLL, Plain, Reader, Source, Stream};
use crate::trim::{self, Trim};
use crate::youtube::segments::Segments;
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

/// How long the stream metadata, and then the preroll, may each take before the attempt is
/// given up. The stream host has no timeout of its own, so a connection it dropped halfway
/// would otherwise hold the engine on a silent track forever.
const PATIENCE: Duration = Duration::from_secs(15);
/// The attempts a fetch gets before the track is reported unavailable.
const ATTEMPTS: u32 = 2;
/// The level the stream host measures `loudnessDb` against, in LUFS.
const REFERENCE_LUFS: f32 = -14.0;

/// A track arriving, with what the stream host said about its length and loudness and what its
/// header says about where the music is.
#[derive(Clone)]
pub struct Loaded {
    stream: Stream,
    /// Where each segment starts. `None` for a file with no index, which a seek then has to
    /// decode its way through.
    segments: Option<Arc<Segments>>,
    /// The edit list, which says how much priming to drop and how long the music runs.
    edit: Option<Trim>,
    loudness_db: Option<f32>,
    duration: Option<Duration>,
    encoding: Encoding,
}

pub struct Factory {
    api: Arc<YtMusic>,
}

impl Factory {
    pub fn new(api: Arc<YtMusic>) -> Self {
        Self { api }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            YouTube {
                api: self.api.clone(),
            },
            config,
        )
    }
}

struct YouTube {
    api: Arc<YtMusic>,
}

impl Source for AudioStream {
    async fn chunk(&mut self) -> Result<Option<Bytes>> {
        AudioStream::chunk(self).await
    }
}

#[async_trait]
impl Fetch for YouTube {
    type Loaded = Loaded;
    type Source = Trimmed<rodio::Decoder<Reader>>;

    fn name(&self) -> &'static str {
        "yt"
    }

    /// Opens the stream and waits for the preroll, giving a stalled attempt one more go before
    /// failing. Only a timeout is retried, since a refusal from the stream host is as final the
    /// second time.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let mut attempt = 1;
        loop {
            match attempt_open(&self.api, id).await {
                Err(error) if attempt < ATTEMPTS && error.is::<tokio::time::error::Elapsed>() => {
                    log::warn!("playback: {id} stalled, trying again: {error:#}");
                    attempt += 1;
                }
                result => return result,
            }
        }
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.loudness_db.map(|db| Loudness {
            lufs: REFERENCE_LUFS + db,
            peak: None,
        })
    }

    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        Some(loaded.encoding)
    }

    fn gated(&self, error: &anyhow::Error) -> bool {
        error.downcast_ref::<ytmusic::SignInRequired>().is_some()
    }

    async fn downloaded(&self, loaded: &Loaded) {
        loaded.stream.finished().await;
    }

    /// A fragmented stream opened unseekable cannot move, so a seek opens a second decoder
    /// spliced to the target.
    fn reopen_to_seek(&self) -> bool {
        true
    }

    /// Builds a decoder over the stream, trimmed to what the edit list says is heard, and
    /// places it at `at`.
    ///
    /// The decoder is told neither the length nor that it may seek. Told either, symphonia walks
    /// every top-level box of the file before the first packet, which waits for the whole
    /// download. Starting anywhere but the beginning splices the header onto the segment that
    /// covers the position, and the part of that segment before it is trimmed off the front.
    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let priming = loaded.edit.map(|edit| edit.skip).unwrap_or_default();
        let entry = match at.is_zero() {
            true => None,
            false => loaded
                .segments
                .as_ref()
                .and_then(|segments| segments.entry(at + priming)),
        };
        let reader = match entry {
            Some(entry) => loaded.stream.spliced(entry.head, entry.graft),
            None => loaded.stream.reader(),
        };
        let decoder = match rodio::Decoder::builder()
            .with_data(reader)
            .with_seekable(false)
            .build()
        {
            Ok(decoder) => decoder,
            Err(error) => {
                log::warn!("playback: cannot decode the youtube track {id}: {error}");
                return None;
            }
        };
        let take = loaded.edit.and_then(|edit| edit.take);
        // Without an index the only way to a position is through every sample before it.
        let (skip, take) = match entry {
            Some(entry) => (entry.into, None),
            None => (priming + at, take.map(|take| take.saturating_sub(at))),
        };
        Some(Trimmed::new(decoder, skip, take))
    }
}

/// Opens the stream and waits for the preroll, each under its own deadline.
async fn attempt_open(api: &Arc<YtMusic>, id: &str) -> Result<Loaded> {
    let started = Instant::now();
    let (format, audio) = tokio::time::timeout(PATIENCE, api.open_audio(id))
        .await
        .context("stream metadata timed out")??;
    let total = audio.total();
    let stream = tokio::time::timeout(PATIENCE, Stream::pulling(audio, total, Plain).primed())
        .await
        .context("stream preroll timed out")??;
    let (segments, edit) = stream
        .with(|_, spool| {
            // The index and the edit list sit at the front, inside what the preroll waited for.
            let front = spool.bytes(0..spool.len().min(PREROLL)).unwrap_or_default();
            (Segments::read(&front), trim::from_mp4(&front))
        })
        .unwrap_or_default();
    log::debug!(
        "playback: {id} started, itag {} {} {} kbps, {:.1} MiB, {} segments, in {:?}",
        format.itag,
        format.codec,
        format.bitrate / 1000,
        total.unwrap_or_default() as f64 / (1024.0 * 1024.0),
        segments.as_ref().map_or(0, Segments::count),
        started.elapsed()
    );
    match edit {
        Some(edit) => log::debug!(
            "playback: {id} trims {:?} of priming, plays {:?}",
            edit.skip,
            edit.take
        ),
        None => log::debug!("playback: {id} carries no edit list"),
    }
    Ok(Loaded {
        stream,
        segments: segments.map(Arc::new),
        edit,
        loudness_db: format.loudness_db,
        duration: format.duration,
        encoding: Encoding {
            codec: match format.codec.as_str() {
                "opus" => Codec::Opus,
                codec if codec.starts_with("mp4a") => Codec::Aac,
                _ => Codec::Other,
            },
            bits: None,
            kbps: (format.bitrate > 0).then_some(format.bitrate / 1000),
        },
    })
}
