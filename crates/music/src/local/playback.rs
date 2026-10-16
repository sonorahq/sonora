//! Playback for files on disk: what [`crate::engine`] needs that is local's own.
//!
//! A track id names a file, so a load only reads its header for the length and its tags for the
//! ReplayGain, and a decoder opens over the file itself. The threads, the queue, the preload, the
//! gapless join and the loudness gain are the engine's.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use async_trait::async_trait;
use rodio::Source as _;

use super::{tags, wire};
use crate::engine::{self, Fetch, Loudness};
use crate::route::Encoding;
use crate::{PlaybackConfig, PlaybackEvents, PlaybackFactory, Player};

/// A file that opened and decodes, how long the decoder says it is, and the ReplayGain its tags
/// carry.
#[derive(Clone)]
pub struct Loaded {
    length: Option<Duration>,
    loudness: Option<Loudness>,
    encoding: Option<Encoding>,
}

#[derive(Default)]
pub struct Factory;

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(Local, config)
    }
}

struct Local;

#[async_trait]
impl Fetch for Local {
    type Loaded = Loaded;
    type Source = rodio::Decoder<BufReader<Audio>>;

    fn name(&self) -> &'static str {
        "local"
    }

    /// Opens the file off the engine's runtime to check it decodes, and reads its length, its
    /// ReplayGain and its encoding while it is there.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let length = decode(&id)?.total_duration();
            let tagged = wire::path_from_track_id(&id).and_then(tags::tagged);
            Ok(Loaded {
                length,
                loudness: tagged.as_ref().and_then(tags::loudness),
                encoding: tagged.as_ref().map(tags::encoding),
            })
        })
        .await
        .context("cannot probe the local file")?
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.length
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.loudness
    }

    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        loaded.encoding
    }

    fn open(&self, id: &str, _loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let mut decoder = match decode(id) {
            Ok(decoder) => decoder,
            Err(error) => {
                log::warn!("playback: cannot decode the local track {id}: {error:#}");
                return None;
            }
        };
        if !at.is_zero()
            && let Err(error) = decoder.try_seek(at)
        {
            log::warn!("playback: cannot start {id} at {}s: {error}", at.as_secs());
        }
        Some(decoder)
    }
}

/// A file read from `skip` on, with every position counted from there, so the decoder never
/// sees the ID3v2 tag in front of the audio. A seek back to the first frame then lands on that
/// frame, not inside the tag, where a cover picture can pass for a frame header.
struct Audio {
    file: File,
    skip: u64,
}

impl Read for Audio {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

impl Seek for Audio {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(at) => SeekFrom::Start(at.saturating_add(self.skip)),
            relative => relative,
        };
        Ok(self.file.seek(pos)?.saturating_sub(self.skip))
    }
}

/// Opens a local file and builds its decoder.
fn decode(id: &str) -> Result<rodio::Decoder<BufReader<Audio>>> {
    let path =
        wire::path_from_track_id(id).ok_or_else(|| anyhow!("{id} is not a local track id"))?;
    let mut file =
        std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let length = file.metadata().ok().map(|meta| meta.len());

    let skip = wire::id3v2_end(path);
    if skip > 0 {
        let _ = file.seek(SeekFrom::Start(skip));
    }
    let gapless = !wire::has_lying_xing_frame_count(path, skip);
    let reader = BufReader::new(Audio { file, skip });

    let mut builder = rodio::Decoder::builder()
        .with_data(reader)
        .with_seekable(true)
        .with_gapless(gapless);

    if let Some(length) = length {
        builder = builder.with_byte_len(length.saturating_sub(skip));
    }
    builder.build().context("cannot decode audio")
}
