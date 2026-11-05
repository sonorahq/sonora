//! Playback for Apple Music: what [`crate::engine`] needs that is Apple's own.
//!
//! The engine, the threads, the queue and the gapless join are shared with every other
//! provider. What is here is where the bytes come from and what a decoder over them looks like.
//! A [`Media`] decrypts each sample through the system Widevine CDM just before it is read, so
//! from `rodio`'s side this is an ordinary clear fMP4, and every sample then travels Sonora's
//! own path: the equalizer, the volume ramp and the spectrum tap in `crate::audio`, out through
//! cpal.
//!
//! One Widevine session holds the keys of more than one track at a time, which is what lets the
//! engine preload: the next track is resolved and licensed while the current one plays, so a
//! queue advance costs nothing.

use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use rodio::Source as _;

use crate::aac::Aac;
use crate::apple::client::AppleClient;
use crate::apple::progressive::{Cenc, Media};
use crate::apple::stream;
use crate::audio::Trimmed;
use crate::engine::{self, Fetch, Loudness};
use crate::stream::Reader;
use crate::trim;
use crate::vault::Shelf;
use crate::{Encoding, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};

/// Priming frames at the head of an Apple AAC encode, before the first frame of music.
///
/// Every AAC encoder writes them, and an MP4 normally says how many in an `elst` box or an
/// `iTunSMPB` tag. Apple's HLS encode carries neither: its init segment is `ftyp` and a `moov`
/// with no `edts` and no `udta` at all. So the figure is the one Apple's own encoder uses, and
/// dropping it is what keeps a join from starting on a click. The tail is cut by playing only
/// as much as the catalog says the track lasts, which is the other half of the same problem.
const PRIMING: u32 = 2112;
/// Priming frames at the head of Apple's HE-AAC encode, at the output rate. That is the core's 2112
/// at twice the rate plus the 962 frames the SBR filterbank delays by, and it lines the 64 kbps
/// stream up with the 256 kbps one to the frame.
const EXTENDED_PRIMING: u32 = 5186;

/// A track resolved, licensed and arriving, with how long and how loud the catalog says it is.
#[derive(Clone)]
pub struct Loaded {
    media: Media,
    duration: Option<Duration>,
    loudness: Option<Loudness>,
    /// The track is the HE-AAC encode, which primes for `EXTENDED_PRIMING` frames.
    extended: bool,
}

pub struct Factory {
    client: AppleClient,
}

impl Factory {
    pub fn new(client: AppleClient) -> Self {
        Self { client }
    }
}

impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(
            Apple {
                client: self.client.clone(),
                quality: config.quality,
                shelf: config.vault.as_ref().map(|vault| vault.shelf("apple")),
            },
            config,
        )
    }
}

struct Apple {
    client: AppleClient,
    /// The level every track is asked for at.
    quality: Quality,
    /// Where played tracks are kept, still encrypted the way Apple sent them.
    shelf: Option<Shelf>,
}

#[async_trait]
impl Fetch for Apple {
    type Loaded = Loaded;
    type Source = Trimmed<rodio::Decoder<Reader<Cenc>>>;

    fn name(&self) -> &'static str {
        "apple"
    }

    fn shelf(&self) -> Option<&Shelf> {
        self.shelf.as_ref()
    }

    /// A kept track is still encrypted, and the key that opens it is licensed afresh for each
    /// play and lasts about fifteen minutes, so a download cannot play without the network.
    fn plays_offline(&self) -> bool {
        false
    }

    /// Resolves and licenses the track, and asks the catalog how long and how loud it is at the
    /// same time.
    /// The length comes back beside the media rather than after it, because the tail trim that
    /// makes a join gapless needs it: the encode itself says nowhere how long the music is,
    /// only how long the file is.
    async fn load(&self, id: &str) -> Result<Loaded> {
        let (prepared, details) = tokio::join!(
            stream::prepare(
                self.client.http(),
                id,
                self.client.user_token(),
                self.quality,
                self.shelf.as_ref(),
            ),
            self.client.playback_details(id),
        );
        let prepared = prepared?;
        Ok(Loaded {
            duration: prepared.duration.or(details.duration),
            extended: prepared.extended,
            loudness: details.loudness,
            media: prepared.media,
        })
    }

    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }

    fn loudness(&self, loaded: &Loaded) -> Option<Loudness> {
        loaded.loudness
    }

    /// Reports the resolved asset's AAC bitrate, including a fallback to the other flavour.
    fn encoding(&self, loaded: &Loaded) -> Option<Encoding> {
        Some(Encoding::Aac {
            kbps: match loaded.extended {
                true => 64,
                false => 256,
            },
        })
    }

    async fn downloaded(&self, loaded: &Loaded) {
        loaded.media.finished().await;
    }

    fn progress(&self, loaded: &Loaded) -> Option<f32> {
        loaded.media.progress()
    }

    /// A fragmented stream has no index for symphonia to jump around in, so a seek opens a
    /// second decoder spliced to the target rather than asking this one to move.
    fn reopen_to_seek(&self) -> bool {
        true
    }

    /// Builds a decoder over a licensed track and places it at `at`. Only the header is read
    /// here; the rest decrypts as it is pulled.
    ///
    /// The track is handed over as a stream, with neither its length nor a seekable source, and
    /// that is a measured choice rather than an oversight. Told the length, symphonia builds a
    /// sample table when it opens the file, and a fragmented stream with no `sidx` leaves it no
    /// way to do that but read the whole track: 5314 of one track's 19378 samples went through
    /// the CDM before the first note, which was 6.8 of the 7.6 seconds it took to start.
    ///
    /// Starting anywhere but the beginning splices instead of winding. Asked to seek, symphonia
    /// decodes its way to the target, about forty milliseconds per second of position. So the
    /// reader hands it the init segment followed by the fragment that covers the position, and
    /// the remainder inside that fragment is trimmed off the front. Nothing before the target
    /// is read at all.
    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let media = &loaded.media;
        let opening = Instant::now();
        let entry = (!at.is_zero()).then(|| media.entry(at)).flatten();
        let reader = match entry {
            Some(entry) => media.reader_at(entry),
            None => media.reader(),
        };
        let decoder = match rodio::Decoder::builder()
            .with_symphonia_decoder::<Aac>()
            .with_data(reader)
            .with_seekable(false)
            .build()
        {
            Ok(decoder) => decoder,
            Err(error) => {
                log::warn!("playback: cannot decode the apple track {id}: {error}");
                return None;
            }
        };

        // The edit list decides what is actually heard, and Apple's encode carries none, so the
        // priming is the encoder's own figure and the tail is the catalog length.
        let edit = media.header().as_deref().and_then(trim::from_mp4);
        let frames = match loaded.extended {
            true => EXTENDED_PRIMING,
            false => PRIMING,
        };
        let priming =
            Duration::from_secs_f64(f64::from(frames) / f64::from(decoder.sample_rate().get()));
        // Spliced, the priming is behind us and the length no longer lines up with what is
        // left, so the only thing to drop is the part of the first fragment before the position.
        let (skip, take) = match entry {
            Some(entry) => (entry.into, None),
            None => (
                edit.map(|edit| edit.skip).unwrap_or(priming),
                edit.and_then(|edit| edit.take).or(loaded.duration),
            ),
        };

        // What the open cost, and how much of that was the CDM. A slow start shows up here
        // first.
        let (samples, decrypting) = media.spent();
        log::info!(
            "apple: decoder started in {:.2}s, {samples} of {} samples decrypted in {:.2}s",
            opening.elapsed().as_secs_f32(),
            media.samples(),
            decrypting.as_secs_f32()
        );
        Some(Trimmed::new(decoder, skip, take))
    }
}
