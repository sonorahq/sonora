//! The output queue every engine feeds. A decoder runs on its own thread and hands finished
//! samples to `Paced`, which appends them to the shared rodio output and blocks the writer once
//! enough is queued. Nothing here decodes or reaches the network, so the audio thread never
//! waits on either.

use std::num::NonZero;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::Result;
use ardftsrc::RealtimeResampler;
use rodio::{ChannelCount, SampleRate, Source};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::audio::{Chain, PublishedFormat};
use crate::output::{self, Output};

/// How many packets stay queued before a write blocks. This is what paces the decoder to real
/// time; deeper rides out network hiccups, but the decoder's position runs this far ahead of
/// what is heard.
const QUEUED_CHUNKS: usize = 26;
/// How many frames each piece of a flushed resampler tail is queued in.
const TAIL_FRAMES: usize = 1024;
/// How many played-out sample buffers are kept for the next packets. A clear retires the whole
/// queue at once, so this covers all of it and refilling after a seek allocates nothing.
const SPARES: usize = QUEUED_CHUNKS + 2;
/// How long a blocked write sleeps between looks at the queue.
const DRAIN_POLL: Duration = Duration::from_millis(10);
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

/// The output device went away, another one is wanted, or its rate changed. The engine reopens
/// rather than carrying on into silence or into a conversion.
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
    format: PublishedFormat,
    born: u32,
    generation: Arc<AtomicU32>,
    played: Arc<AtomicU64>,
    live: Arc<AtomicUsize>,
    spares: Spares,
}

impl Chunk {
    fn new(
        packet: Packet,
        format: &PublishedFormat,
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
    /// What is left of the packet, and nothing once a clear has retired it. rodio reads the
    /// format again only where a span ends, so a span that ran on past this packet would play
    /// the next one in this one's channel count.
    ///
    /// Note a rodio bug: rodio's trait docs ask for the whole span, but its queue and mixer
    /// read this as what is left, so thats what we do here.
    ///
    /// TODO: When updating rodio, check if this behavior changed, and adjust this code accordingly.
    fn current_span_len(&self) -> Option<usize> {
        match self.generation.load(Ordering::Relaxed) == self.born {
            true => Some(self.samples.len() - self.at),
            false => Some(0),
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
/// queued, which keeps a decoder in step with playback. A write on an output that has to reopen
/// reports it gone; the output also says so on `changed` itself, playing or not.
///
/// Every packet goes through the resampler as it is written, so everything queued is at the
/// device's rate already and a pause or a clear acts on it at once. A packet at the device's rate
/// is copied through as it is, and a change of rate or channels starts a new span, which keeps
/// the order of what is written without a flush.
pub struct Paced {
    output: Output,
    cue: Cue,
    /// Chunks appended and not yet played out or retired.
    live: Arc<AtomicUsize>,
    spares: Spares,
    changed: UnboundedSender<()>,
    /// ardftsrc at its good preset, for every writer.
    resampler: RealtimeResampler,
    /// The cue generation the audio the resampler holds belongs to.
    generation: u32,
}

impl Paced {
    /// Claims the output device, paused until the caller starts it.
    pub fn open(cue: Cue, chain: Chain, changed: UnboundedSender<()>) -> Result<Self> {
        let output = Output::open(chain, changed.clone())?;
        output.sink().pause();

        // the input format is a placeholder: each packet starts a span in its own
        let rate = output.rate() as usize;
        let config = ardftsrc::PRESET_GOOD
            .with_input_rate(rate)
            .with_output_rate(rate)
            .with_channels(2);
        // rodio refuses a zero rate when the output opens, so the config is always valid
        let resampler = RealtimeResampler::new(config)
            .expect("PRESET_GOOD at the device's non-zero rate, in stereo, is a valid config");
        let generation = cue.generation.load(Ordering::Relaxed);
        Ok(Self {
            output,
            cue,
            live: Arc::default(),
            spares: Spares::new(),
            changed,
            resampler,
            generation,
        })
    }

    pub fn play(&mut self) -> Result<(), Gone> {
        if self.output.failed() {
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
    ///
    /// Returns how many samples, at the device's rate, it added to the queue. Resampling holds
    /// some audio back until more is written or `flush` lets it out, so that is not always the
    /// packet's length.
    pub fn write(&mut self, packet: Packet) -> Result<usize, Gone> {
        if self.output.failed() {
            return Err(self.disconnected());
        }
        if !self.cue.admit() {
            self.spares.keep(packet.samples);
            return Ok(0);
        }

        self.forget_cleared();
        let device = self.rate();
        let rate = packet.rate.get();
        let resampler = &mut self.resampler;
        let channels = usize::from(packet.channels.get());

        // pieces about as long as the packet, so a resampled queue paces like the packets did
        let frames = packet.samples.len() / channels;
        let piece = (frames as u64 * u64::from(device)).div_ceil(u64::from(rate)) as usize;
        let written = resampler.write_span(
            rate as usize,
            channels,
            packet.samples.iter().map(|&sample| f64::from(sample)),
        );
        // the resampler has copied what it needs, so the buffer goes round for the next packet
        self.spares.keep(packet.samples);
        if let Err(error) = written {
            log::error!("sink: cannot resample a packet: {error}");
            resampler.reset();
            return Ok(0);
        }
        Ok(self.queue_ready(piece))
    }

    /// Whether the queue holds all it should, so the next packet can wait.
    pub fn full(&self) -> bool {
        self.live.load(Ordering::Relaxed) > QUEUED_CHUNKS
    }

    /// Flush the resampler. This ends the stream, so it is for when nothing follows: the end of
    /// the queue, or a writer that stops.
    pub fn flush(&mut self) -> usize {
        self.forget_cleared();
        let resampler = &mut self.resampler;
        if resampler.is_empty() {
            return 0;
        }
        if self.cue.cleared() {
            resampler.reset();
            return 0;
        }
        if let Err(error) = resampler.finalize() {
            log::error!("sink: cannot flush the resampler: {error}");
            resampler.reset();
            return 0;
        }
        self.queue_ready(TAIL_FRAMES)
    }

    /// Samples already written that the resampler has yet to queue, at the device's rate. Added
    /// to what the writes returned, it is where the next write starts, to within a frame.
    pub fn held_back(&self) -> usize {
        self.resampler.samples_outstanding()
    }

    /// The rate the device runs at. A packet at any other rate is resampled before it is heard.
    pub fn rate(&self) -> u32 {
        self.output.rate()
    }

    /// Waits until there is room for another packet.
    pub fn drain(&mut self) -> Result<(), Gone> {
        while self.full() {
            if self.output.failed() {
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

    /// Tells the engine the output is gone.
    pub fn disconnected(&self) -> Gone {
        self.changed.send(()).ok();
        Gone
    }

    /// Queues what the resampler has ready in pieces of `frames` frames, and returns how many
    /// samples that added. A partial piece waits for the next write (except the tail of a span,
    /// which is queued as it is).
    fn queue_ready(&mut self, frames: usize) -> usize {
        let mut queued = 0;
        while let Some(span) = self.resampler.read_span(frames) {
            let (channels, rate) = (span.channels() as u16, span.sample_rate() as u32);
            let mut samples = self.spares.take(span.len());
            samples.extend(span.map(|sample| sample as f32));
            let Some(packet) = packet(samples, channels, rate) else {
                continue;
            };
            queued += packet.samples.len();
            let format = self.output.format();
            let chunk = Chunk::new(packet, format, &self.cue, &self.live, &self.spares);
            self.output.sink().append(chunk);
        }
        queued
    }

    /// Drops what the resampler holds once the cue has moved on a generation, since that is the
    /// old position, which is not heard.
    fn forget_cleared(&mut self) {
        let generation = self.cue.generation.load(Ordering::Relaxed);
        if generation == self.generation {
            return;
        }
        self.generation = generation;
        self.resampler.reset();
    }
}

/// Waits for an output device to come back and then tells `notify`, which the engine takes the
/// way it takes the device changing: it reopens the output and reloads where it was.
/// This is what an engine that could not open a device at all waits on. A machine whose only
/// device has gone, a Bluetooth headset that dropped its link, has nothing to reopen on and
/// nothing else asks the system again, so without the watch playback stays silent until the app
/// restarts. The watch ends when the engine listening on `notify` is gone, so the caller has to
/// keep that engine alive while it waits, and an engine's audio thread does that by staying up.
///
/// A device that is present is not enough: ALSA always names a default one, and without the
/// sound server's plugin it never opens. Reporting it anyway would restart the engine every poll,
/// and dropping librespot's player joins its thread on the main one, so the watch waits until a
/// stream actually opens.
pub fn watch_for_output(notify: UnboundedSender<()>) {
    std::thread::spawn(move || {
        while !notify.is_closed() {
            std::thread::sleep(WATCH_POLL);
            if !output::output_openable() {
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
    use super::*;
    use crate::audio::{Meter, Volume};
    use crate::equalizer::{Curve, EqualizerWriter};
    use crate::spectrum::{Analyzer, Spectrum};

    /// A stereo track comes out the same whichever way the mono one before it stopped: on its
    /// own, or cut short by a skip with more of its packets still queued. A sample counted into
    /// the wrong frame carries the mono track's filter state into the stereo one.
    #[test]
    fn a_new_track_starts_on_a_fresh_frame() {
        let (cue, _) = Cue::new();
        let (live, spares) = (Arc::default(), Spares::new());
        let append =
            |player: &rodio::Player, format: &PublishedFormat, samples: &[f32], channels| {
                let packet = packet(samples.to_vec(), channels, 48_000).unwrap();
                player.append(Chunk::new(packet, format, &cue, &live, &spares));
            };
        let tone = |i: usize| (i as f32 * 0.05).sin();
        let mono: Vec<f32> = (0..256).map(tone).collect();
        let stereo: Vec<f32> = (1..1_025).flat_map(|i| [tone(i), 0.]).collect();

        let (player, chain, format) = open();
        append(&player, &format, &stereo, 2);
        let alone: Vec<f32> = chain.take(stereo.len()).collect();

        let (player, chain, format) = open();
        append(&player, &format, &mono[..3], 1);
        append(&player, &format, &stereo, 2);
        let ended: Vec<f32> = chain.skip(3).take(stereo.len()).collect();

        let (player, mut chain, format) = open();
        for _ in 0..3 {
            append(&player, &format, &mono, 1);
        }
        chain.by_ref().take(32).for_each(drop);
        cue.clear();
        append(&player, &format, &stereo, 2);
        let skipped: Vec<f32> = chain.take(stereo.len()).collect();

        for (case, heard) in [("ended", ended), ("skipped", skipped)] {
            assert!(
                heard == alone,
                "{case}: {:?} for {:?}",
                &heard[..6],
                &alone[..6]
            );
        }
    }

    /// A player and the chain its queue plays through, as an output opens them.
    fn open() -> (rodio::Player, impl Iterator<Item = f32>, PublishedFormat) {
        let (player, queue) = rodio::Player::new();
        let format = PublishedFormat::default();
        let writer = EqualizerWriter::new(true, Curve::bands(&[12.; 10]));
        let chain = Chain {
            volume: Volume::new(0.5),
            effects: crate::Effects::default(),
            equalizer: writer.reader(),
            spectrum: Spectrum::new(),
            meter: Meter::default(),
            analyzer: Analyzer::default(),
        };
        (player, chain.apply(queue, format.clone(), false), format)
    }
}
