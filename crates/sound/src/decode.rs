//! The decoders the engine's audio thread pulls samples from.
//!
//! Generic code is compiled in the crate that names its types, at that crate's optimisation
//! level. Providers live in crates built without the sound path's optimisations, so a decoder over
//! a provider's own reader type would decode every sample in an unoptimised loop. [`Media`] puts
//! any reader behind one type instead, and [`Decoded`] holds the decoder over it, so the decoder
//! and the loop that drains it are compiled here, whoever opened them.

use std::io::{self, IoSliceMut, Read, Seek, SeekFrom};
use std::time::Duration;

use rodio::Source;
use rodio::source::SeekError;

use crate::audio::Trimmed;

/// A provider's reader, put behind one type so the decoder over it is the same type for every
/// provider. Every read and seek goes straight through to the reader.
pub struct Media(Box<dyn Readable>);

/// What a decoder reads from.
trait Readable: Read + Seek + Send + Sync {}

impl<T: Read + Seek + Send + Sync> Readable for T {}

impl Media {
    pub fn new(reader: impl Read + Seek + Send + Sync + 'static) -> Self {
        Self(Box::new(reader))
    }
}

impl Read for Media {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.0.read_vectored(bufs)
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        self.0.read_to_end(buf)
    }

    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        self.0.read_to_string(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.0.read_exact(buf)
    }
}

impl Seek for Media {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        self.0.stream_position()
    }

    fn seek_relative(&mut self, offset: i64) -> io::Result<()> {
        self.0.seek_relative(offset)
    }
}

/// A track's decoder as the engine plays it: every sample the decoder yields, or the part of
/// them an MP4 edit list says should be heard.
pub struct Decoded(Inner);

enum Inner {
    Whole(rodio::Decoder<Media>),
    Trimmed(Trimmed<rodio::Decoder<Media>>),
}

impl Decoded {
    /// Every sample `decoder` yields.
    pub fn new(decoder: rodio::Decoder<Media>) -> Self {
        Self(Inner::Whole(decoder))
    }

    /// The samples of `decoder` from `skip` on, and only the `take` that follow when it is
    /// given, as [`Trimmed`] cuts them.
    pub fn trimmed(decoder: rodio::Decoder<Media>, skip: Duration, take: Option<Duration>) -> Self {
        Self(Inner::Trimmed(Trimmed::new(decoder, skip, take)))
    }

    pub fn channels(&self) -> u16 {
        match &self.0 {
            Inner::Whole(decoder) => decoder.channels().get(),
            Inner::Trimmed(trimmed) => trimmed.channels().get(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        match &self.0 {
            Inner::Whole(decoder) => decoder.sample_rate().get(),
            Inner::Trimmed(trimmed) => trimmed.sample_rate().get(),
        }
    }

    pub fn try_seek(&mut self, position: Duration) -> Result<(), SeekError> {
        match &mut self.0 {
            Inner::Whole(decoder) => decoder.try_seek(position),
            Inner::Trimmed(trimmed) => trimmed.try_seek(position),
        }
    }

    /// Pulls up to `wanted` samples onto the end of `samples`, each multiplied by `gain`, and
    /// hands the buffer back. Fewer than `wanted` means the track ended.
    pub fn take(&mut self, wanted: usize, gain: f32, samples: Vec<f32>) -> Vec<f32> {
        match &mut self.0 {
            Inner::Whole(decoder) => fill(decoder, wanted, gain, samples),
            Inner::Trimmed(trimmed) => fill(trimmed, wanted, gain, samples),
        }
    }
}

/// [`Decoded::take`] for one kind of source.
fn fill(source: &mut impl Source, wanted: usize, gain: f32, mut samples: Vec<f32>) -> Vec<f32> {
    for sample in source.by_ref().take(wanted) {
        samples.push(sample * gain);
    }
    samples
}
