//! What one engine knows about the sound it plays, for the signal path: the format the music
//! reaches the chain in, and for each track how it is encoded and what normalisation does to it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// How many tracks' facts are kept: the one playing, the one queued behind it, and room for a
/// skip that has not been heard yet.
const TRACKS: usize = 4;

/// Shared by an engine, its outputs and the UI, like the equalizer's reader.
#[derive(Clone, Default)]
pub struct Route(Arc<Shared>);

#[derive(Default)]
struct Shared {
    /// The format the chain last took up, `rate << 16 | channels`, zero before the first.
    heard: AtomicU64,
    /// Newest last.
    tracks: Mutex<VecDeque<(String, TrackSignal)>>,
}

/// What an engine knows about one track.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackSignal {
    /// How the provider encodes it, when it says.
    pub encoding: Option<Encoding>,
    pub normalisation: Normalisation,
}

/// What loudness normalisation does to a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Normalisation {
    Off,
    /// A fixed gain in decibels, zero when nothing says how loud the track is.
    Gain(f32),
    /// librespot normalises inside its own player with a limiter, so the gain is not known
    /// and changes with the music.
    Limited,
}

/// How a track is encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encoding {
    pub codec: Codec,
    /// The bits per sample of a lossless encoding.
    pub bits: Option<u8>,
    /// The bit rate of a lossy encoding, in kilobits a second.
    pub kbps: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Flac,
    Alac,
    /// Uncompressed, as in WAV or AIFF.
    Pcm,
    Mp3,
    Aac,
    Vorbis,
    Opus,
    Other,
}

impl Route {
    /// The rate and channel count the chain last took up, `None` before anything played.
    pub fn heard(&self) -> Option<(u32, u16)> {
        let word = self.0.heard.load(Ordering::Relaxed);
        (word != 0).then_some(((word >> 16) as u32, word as u16))
    }

    /// What is known about track `id`, if it played or is about to.
    pub fn track(&self, id: &str) -> Option<TrackSignal> {
        let tracks = self.0.tracks.lock().unwrap_or_else(PoisonError::into_inner);
        tracks
            .iter()
            .rev()
            .find(|(known, _)| known == id)
            .map(|(_, track)| track.clone())
    }

    /// Notes what the chain takes up. Called on the audio thread at a format change only.
    pub(crate) fn hear(&self, rate: u32, channels: u16) {
        let word = u64::from(rate) << 16 | u64::from(channels);
        self.0.heard.store(word, Ordering::Relaxed);
    }

    /// Notes what track `id` is, as it starts decoding. Never called from the audio callback.
    pub(crate) fn learn(&self, id: &str, track: TrackSignal) {
        let mut tracks = self.0.tracks.lock().unwrap_or_else(PoisonError::into_inner);
        tracks.retain(|(known, _)| known != id);
        if tracks.len() == TRACKS {
            tracks.pop_front();
        }
        tracks.push_back((id.to_owned(), track));
    }
}
