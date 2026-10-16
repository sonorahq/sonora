//! The output queue every engine feeds. A decoder runs on its own thread and hands finished
//! samples to `Paced`, which appends them to the shared rodio output and blocks the writer once
//! enough is queued. Nothing here decodes or reaches the network, so the audio thread never
//! waits on either.

use std::num::NonZero;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Result;
use rodio::{ChannelCount, SampleRate, Source};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::audio::{Chain, Format, Output};

/// How many packets stay queued before a write blocks. This is what paces the decoder to real
/// time; deeper rides out network hiccups, but the decoder's position runs this far ahead of
/// what is heard.
const QUEUED_CHUNKS: usize = 26;
/// How many played-out sample buffers are kept for the next packets. A clear retires the whole
/// queue at once, so this covers all of it and refilling after a seek allocates nothing.
const SPARES: usize = QUEUED_CHUNKS + 2;
/// How long a blocked write sleeps between looks at the queue.
const DRAIN_POLL: Duration = Duration::from_millis(10);
/// How often a write asks the system whether the default device changed.
const DEVICE_POLL: Duration = Duration::from_millis(500);
/// How often a watch looks for an output device to come back once none could be opened.
const WATCH_POLL: Duration = Duration::from_secs(1);

/// What the cue does with the next write. It rides in an `AtomicU8`, so it converts at that
/// boundary and stays a type everywhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum Admit {
    /// Writes pass through and nothing is reported.
    Open,
    /// A clear was asked for and the engine has not announced the new track or position yet,
    /// so every write is still the old one and is dropped.
    Cleared,
    /// The engine announced the new track or position; the next write is its first audio.
    Armed,
}

impl Admit {
    fn of(value: u8) -> Self {
        match value {
            1 => Self::Cleared,
            2 => Self::Armed,
            _ => Self::Open,
        }
    }
}

/// The output device went away, or the default one changed. The engine reopens rather than
/// carrying on into silence.
#[derive(Debug)]
pub struct Gone;

impl std::fmt::Display for Gone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "audio output changed")
    }
}

impl std::error::Error for Gone {}

/// The queue's link to the event stream. `clear` retires every packet queued so far and refuses
/// the ones a decoder still writes from the old position, so the old audio stops when the user
/// acts. `arm` lets writes through again and has the first one reported, which is how a provider
/// marks audio from the new track or position rather than a decoder merely being ready.
#[derive(Clone)]
pub struct Cue {
    state: Arc<AtomicU8>,
    generation: Arc<AtomicU32>,
    /// Samples that have reached the output since the last clear.
    played: Arc<AtomicU64>,
    written: UnboundedSender<()>,
}

impl Cue {
    pub fn new() -> (Self, UnboundedReceiver<()>) {
        let (written, receiver) = unbounded_channel();
        let cue = Self {
            state: Arc::default(),
            generation: Arc::default(),
            played: Arc::default(),
            written,
        };
        (cue, receiver)
    }

    /// Moves the queue on a generation. Every chunk from before ends at its next sample, and
    /// nothing here touches rodio's own bookkeeping, which does not survive a clear that races
    /// a chunk ending on its own.
    pub fn clear(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.state.store(Admit::Cleared as u8, Ordering::Relaxed);
        self.played.store(0, Ordering::Relaxed);
    }

    /// Lets writes through again and has the next one reported.
    pub fn arm(&self) {
        self.state.store(Admit::Armed as u8, Ordering::Relaxed);
    }

    /// Whether a clear is still waiting for the engine to announce where it went.
    pub fn cleared(&self) -> bool {
        self.state() == Admit::Cleared
    }

    fn state(&self) -> Admit {
        Admit::of(self.state.load(Ordering::Relaxed))
    }

    /// Samples heard since the last clear, across every channel.
    pub fn played(&self) -> u64 {
        self.played.load(Ordering::Relaxed)
    }

    /// Decides a write: refused after a clear, reported when armed, plain otherwise. A clear
    /// that lands during the armed write wins, since that packet predates the seek behind it.
    pub(crate) fn admit(&self) -> bool {
        match self.state() {
            Admit::Cleared => false,
            Admit::Armed => {
                let armed = self
                    .state
                    .compare_exchange(
                        Admit::Armed as u8,
                        Admit::Open as u8,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .is_ok();
                if armed {
                    self.written.send(()).ok();
                }
                armed
            }
            Admit::Open => true,
        }
    }
}

/// A packet of interleaved samples at one rate, ready for the queue.
pub struct Packet {
    samples: Vec<f32>,
    channels: ChannelCount,
    rate: SampleRate,
}

/// Sample buffers that have played out, handed back to the decoder for its next packets, so
/// steady playback reuses the same few buffers instead of allocating one per packet. The output
/// callback only ever tries the lock, so it never waits on the decoder.
#[derive(Clone)]
struct Spares(Arc<Mutex<Vec<Vec<f32>>>>);

impl Spares {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::with_capacity(SPARES))))
    }

    /// An empty buffer with room for `wanted` samples, a spare one when there is one.
    fn take(&self, wanted: usize) -> Vec<f32> {
        let spare = self.0.lock().unwrap_or_else(PoisonError::into_inner).pop();
        let mut buffer = spare.unwrap_or_default();
        buffer.clear();
        buffer.reserve(wanted);
        buffer
    }

    /// Keeps `buffer` for a later packet. It is freed instead when the lock is busy or enough
    /// are kept already, and keeping one never allocates, because the list is sized up front.
    fn keep(&self, buffer: Vec<f32>) {
        if let Ok(mut spares) = self.0.try_lock()
            && spares.len() < SPARES
            && buffer.capacity() > 0
        {
            spares.push(buffer);
        }
    }
}

/// One packet in the output queue. It ends as soon as the cue moves on a generation and counts
/// itself out when dropped, whichever way it went, so `live` is the number still queued. Its
/// buffer goes back to the spares when it drops.
struct Chunk {
    samples: Vec<f32>,
    /// The next sample to hand out.
    at: usize,
    channels: ChannelCount,
    rate: SampleRate,
    /// Where the chunk announces its format to the chain as it starts playing.
    format: Format,
    born: u32,
    generation: Arc<AtomicU32>,
    played: Arc<AtomicU64>,
    live: Arc<AtomicUsize>,
    spares: Spares,
}

impl Chunk {
    fn new(
        packet: Packet,
        format: &Format,
        cue: &Cue,
        live: &Arc<AtomicUsize>,
        spares: &Spares,
    ) -> Self {
        live.fetch_add(1, Ordering::Relaxed);
        Self {
            samples: packet.samples,
            at: 0,
            channels: packet.channels,
            rate: packet.rate,
            format: format.clone(),
            born: cue.generation.load(Ordering::Relaxed),
            generation: cue.generation.clone(),
            played: cue.played.clone(),
            live: live.clone(),
            spares: spares.clone(),
        }
    }
}

impl Iterator for Chunk {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.generation.load(Ordering::Relaxed) != self.born {
            return None;
        }
        let sample = *self.samples.get(self.at)?;
        if self.at == 0 {
            self.format.publish(self.rate, self.channels);
        }
        self.at += 1;
        self.played.fetch_add(1, Ordering::Relaxed);
        Some(sample)
    }
}

impl Source for Chunk {
    /// The whole packet until its last sample is out, then nothing, the way a rodio
    /// `SamplesBuffer` answers.
    fn current_span_len(&self) -> Option<usize> {
        match self.at >= self.samples.len() {
            true => Some(0),
            false => Some(self.samples.len()),
        }
    }

    fn channels(&self) -> ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> SampleRate {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        let nanos = 1_000_000_000u64.checked_mul(self.samples.len() as u64)?
            / u64::from(self.rate.get())
            / u64::from(self.channels.get());
        Some(Duration::from_nanos(nanos))
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
        self.spares.keep(std::mem::take(&mut self.samples));
    }
}

/// The shared rodio output with a paced queue in front of it. A write blocks once enough is
/// queued, which keeps a decoder in step with playback, and every write checks that the default
/// device is still the one the output was opened on.
pub struct Paced {
    output: Output,
    cue: Cue,
    /// Chunks appended and not yet played out or retired.
    live: Arc<AtomicUsize>,
    spares: Spares,
    changed: UnboundedSender<()>,
    checked_at: Instant,
}

impl Paced {
    /// Claims the default output device, paused until the caller starts it.
    pub fn open(cue: Cue, chain: Chain, changed: UnboundedSender<()>) -> Result<Self> {
        let output = Output::open(chain)?;
        output.sink().pause();

        Ok(Self {
            output,
            cue,
            live: Arc::default(),
            spares: Spares::new(),
            changed,
            checked_at: Instant::now(),
        })
    }

    pub fn play(&mut self) -> Result<(), Gone> {
        if self.output.failed() || self.output.changed() {
            return Err(self.disconnected());
        }
        self.output.sink().play();
        Ok(())
    }

    pub fn pause(&self) {
        self.output.sink().pause();
    }

    pub fn set_volume(&self, gain: f32) {
        self.output.set_volume(gain);
    }

    /// An empty sample buffer with room for `wanted` samples, for the next packet. It is one
    /// that has played out when there is one, so filling the queue in step with the output
    /// allocates nothing.
    pub fn buffer(&self, wanted: usize) -> Vec<f32> {
        self.spares.take(wanted)
    }

    /// Queues one packet. A packet the cue refuses is dropped, so a decoder that has been
    /// retired makes no sound while it winds down. This never blocks: a caller with nothing
    /// else to do waits through `drain`, one with commands to answer waits through `full`.
    pub fn write(&mut self, packet: Packet) -> Result<(), Gone> {
        if self.output_changed() {
            return Err(self.disconnected());
        }
        if !self.cue.admit() {
            self.spares.keep(packet.samples);
            return Ok(());
        }

        let chunk = Chunk::new(
            packet,
            self.output.format(),
            &self.cue,
            &self.live,
            &self.spares,
        );
        self.output.sink().append(chunk);
        Ok(())
    }

    /// Whether the queue holds all it should, so the next packet can wait.
    pub fn full(&self) -> bool {
        self.live.load(Ordering::Relaxed) > QUEUED_CHUNKS
    }

    /// Whether everything queued has played out or been retired. A queue on a stream that has
    /// failed or closed never plays out, so it counts as drained.
    pub fn drained(&self) -> bool {
        self.live.load(Ordering::Relaxed) == 0 || self.output.failed()
    }

    /// Whether packets at `rate` can go out without the output reopening.
    pub fn fits(&self, rate: u32) -> bool {
        self.output.fits(rate)
    }

    /// Reopens the output for packets at `rate` unless it fits already. Anything still queued
    /// is dropped, so a caller that wants the tail heard waits for `drained` first.
    pub fn fit(&mut self, rate: u32) -> Result<(), Gone> {
        match self.output.fit(rate) {
            Ok(_) => Ok(()),
            Err(error) => {
                log::error!("sink: cannot reopen the audio output: {error:#}");
                Err(self.disconnected())
            }
        }
    }

    /// Waits until there is room for another packet.
    pub fn drain(&mut self) -> Result<(), Gone> {
        while self.full() {
            if self.output_changed() {
                return Err(self.disconnected());
            }
            std::thread::sleep(DRAIN_POLL);
        }
        Ok(())
    }

    /// How long a full queue takes to play out. This is how far a decoder runs ahead of what
    /// is heard, so it is also the longest a paused writer waits before it sees a command.
    pub const fn poll() -> Duration {
        DRAIN_POLL
    }

    /// Whether the output failed or the default device is no longer the one it was opened on,
    /// asking the system at most every `DEVICE_POLL`.
    pub fn output_changed(&mut self) -> bool {
        let now = Instant::now();
        let changed = self.output.failed()
            || now.duration_since(self.checked_at) >= DEVICE_POLL && self.output.changed();
        if now.duration_since(self.checked_at) >= DEVICE_POLL {
            self.checked_at = now;
        }
        changed
    }

    /// Tells the engine the output is gone.
    pub fn disconnected(&self) -> Gone {
        self.changed.send(()).ok();
        Gone
    }
}

/// Waits for an output device to come back and then tells `notify`, which the engine takes the
/// way it takes the default device changing: it reopens the output and reloads where it was.
/// This is what an engine that could not open a device at all waits on. A machine whose only
/// device has gone, a Bluetooth headset that dropped its link, has nothing to reopen on and
/// nothing else asks the system again, so without the watch playback stays silent until the app
/// restarts. The watch ends when the engine listening on `notify` is gone.
///
/// A device that is present is not enough: ALSA always names a default one, and without the
/// sound server's plugin it never opens. Reporting it anyway would restart the engine every poll,
/// and dropping librespot's player joins its thread on the main one, so the watch waits until a
/// stream actually opens.
pub fn watch_for_output(notify: UnboundedSender<()>) {
    std::thread::spawn(move || {
        while !notify.is_closed() {
            std::thread::sleep(WATCH_POLL);
            if !crate::audio::openable() {
                continue;
            }
            log::info!("sink: an output device is available again, reopening");
            notify.send(()).ok();
            return;
        }
    });
}

/// A packet of interleaved samples at one rate, ready for the queue. It takes the buffer over
/// rather than copying it, so a buffer from [`Paced::buffer`] goes round without allocating.
pub fn packet(samples: Vec<f32>, channels: u16, rate: u32) -> Option<Packet> {
    Some(Packet {
        samples,
        channels: NonZero::new(channels)?,
        rate: NonZero::new(rate)?,
    })
}

#[cfg(test)]
mod tests {
    use rodio::queue::SourcesQueueOutput;

    use super::*;
    use crate::audio::{Frames, SmoothGain, Volume};
    use crate::equalizer::{BANDS, Equalized, Equalizer};

    /// A named sequence of pushes, pulls, clears and pauses run against a fresh rig.
    type Scenario = (&'static str, fn(&mut Rig));

    /// A player whose queue is read the way a chain stage reads it. Every pushed sample is a
    /// distinct number indexing the place it should land on, so a misplaced one names itself.
    struct Rig {
        name: &'static str,
        player: rodio::Player,
        queue: SourcesQueueOutput,
        format: Format,
        frames: Frames,
        cue: Cue,
        live: Arc<AtomicUsize>,
        spares: Spares,
        /// The channel, channel count and rate expected for the sample `i + 1`.
        expected: Vec<(u16, u16, u32)>,
    }

    impl Rig {
        fn new(name: &'static str) -> Self {
            let (player, queue) = rodio::Player::new();
            let format = Format::default();
            Self {
                name,
                player,
                queue,
                frames: Frames::new(format.clone()),
                format,
                cue: Cue::new().0,
                live: Arc::default(),
                spares: Spares::new(),
                expected: Vec::new(),
            }
        }

        fn append(&self, samples: Vec<f32>, channels: u16, rate: u32) {
            let packet = packet(samples, channels, rate).unwrap();
            let chunk = Chunk::new(packet, &self.format, &self.cue, &self.live, &self.spares);
            self.player.append(chunk);
        }

        fn push(&mut self, channels: u16, rate: u32, samples: usize) {
            let first = self.expected.len();
            self.expected
                .extend((0..samples).map(|i| (i as u16 % channels, channels, rate)));
            let numbered = (first..first + samples).map(|i| (i + 1) as f32);
            self.append(numbered.collect(), channels, rate);
        }

        fn pull(&mut self, count: usize) {
            for _ in 0..count {
                self.step();
            }
        }

        /// Pulls until `count` pushed samples have come out, through any zeros in between.
        fn until(&mut self, count: usize) {
            let mut left = count;
            for _ in 0..20_000 {
                if left == 0 {
                    return;
                }
                left -= usize::from(self.step());
            }
            panic!("{}: {left} samples never came out", self.name);
        }

        /// Pulls one sample and checks where the frame counter put it, if it was a pushed one.
        fn step(&mut self) -> bool {
            let (sample, place) = self.frames.pull(&mut self.queue).unwrap();
            if sample == 0. {
                return false;
            }
            let place = place.unwrap_or_else(|| panic!("{}: {sample} has no format", self.name));
            let found = (place.channel, self.frames.channels(), self.frames.rate());
            let expected = self.expected[sample as usize - 1];
            assert_eq!(found, expected, "{}: sample {sample}", self.name);
            true
        }
    }

    #[test]
    fn every_sample_keeps_its_channel() {
        let scenarios: [Scenario; 4] = [
            ("mono retired by a clear, stereo follows", |rig| {
                rig.push(1, 48_000, 6);
                rig.pull(2);
                rig.cue.clear();
                rig.push(2, 44_100, 4);
                rig.until(4);
            }),
            ("stereo retired mid-frame, stereo follows", |rig| {
                rig.push(2, 44_100, 6);
                rig.pull(3);
                rig.cue.clear();
                rig.push(2, 44_100, 4);
                rig.until(4);
            }),
            ("paused between a stereo chunk and a mono one", |rig| {
                rig.push(2, 44_100, 4);
                rig.push(1, 88_200, 4);
                rig.until(4);
                rig.player.pause();
                rig.pull(3);
                rig.player.play();
                rig.until(4);
            }),
            ("paused mid-frame inside a stereo chunk", |rig| {
                rig.push(2, 44_100, 1_000);
                // rodio applies controls every 5 ms, 441 samples here, so the pause lands
                // between the left and right samples of a frame.
                rig.until(441);
                rig.player.pause();
                assert!(
                    !rig.step(),
                    "{}: the pause did not land mid-frame",
                    rig.name
                );
                rig.pull(2);
                rig.player.play();
                rig.until(559);
            }),
        ];
        for (name, run) in scenarios {
            run(&mut Rig::new(name));
        }
    }

    #[test]
    fn silent_right_channel_stays_silent() {
        let rig = Rig::new("tone on the left");
        let tone = |i: usize| (i as f32 * 0.05).sin();
        rig.append((0..3).map(tone).collect(), 1, 44_100);
        rig.append((0..2_048).flat_map(|i| [tone(i), 0.]).collect(), 2, 44_100);

        let Rig {
            player: _player,
            queue,
            format,
            ..
        } = rig;
        let equalizer = Equalizer::new(true, &[12.; BANDS]);
        let equalized = Equalized::new(queue, equalizer, format.clone());
        let chain = SmoothGain::new(equalized, Volume::new(0.5), 0.5, Duration::ZERO, format);
        let right: Vec<f32> = chain.skip(3).take(4_096).skip(1).step_by(2).collect();
        assert!(
            right.iter().all(|sample| sample.to_bits() == 0),
            "{:?}",
            &right[..8]
        );
    }
}
