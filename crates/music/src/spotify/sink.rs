//! librespot's `Sink` over the shared paced output. librespot decodes on its own thread, so
//! every write here is finished audio; the pacing lives in `crate::sink` and the device watch in
//! `crate::output`.

use std::time::Duration;

use librespot_playback::audio_backend::{Sink, SinkError, SinkResult};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use tokio::sync::mpsc::UnboundedSender;

use crate::audio::Chain;
use crate::sink::{Cue, Paced, packet, watch_for_output};

pub struct OutputSink {
    paced: Paced,
    /// Set once the output has to reopen, so that is logged once rather than for every packet.
    reopening: bool,
}

impl OutputSink {
    /// The sink librespot's player builder asks for. Without an output device it gets a silent
    /// one, so playback state still moves.
    pub fn boxed(cue: Cue, chain: Chain, changed: UnboundedSender<()>) -> Box<dyn Sink> {
        match Paced::open(cue.clone(), chain, changed.clone()) {
            Ok(paced) => Box::new(Self {
                paced,
                reopening: false,
            }),
            Err(error) => {
                log::error!("sink: cannot open an output device: {error:#}");
                watch_for_output(changed);
                Box::new(Silence(cue))
            }
        }
    }

    /// Notes that the output has to reopen. `Paced` has told the engine already, which rebuilds
    /// the player on the new output, so librespot is not handed an error: it would log one and
    /// pause, for what is a device change rather than a failure.
    fn reopen(&mut self) {
        if !std::mem::replace(&mut self.reopening, true) {
            log::info!("sink: the audio output changed, waiting for the player to be rebuilt");
        }
    }
}

impl Sink for OutputSink {
    fn start(&mut self) -> SinkResult<()> {
        if self.paced.play().is_err() {
            self.reopen();
        }
        Ok(())
    }

    /// librespot stops the sink for a pause and at the end of playback alike, so what the
    /// resampler holds is let out either way, or the end of the last track would be lost.
    fn stop(&mut self) -> SinkResult<()> {
        self.paced.flush();
        self.paced.pause();
        Ok(())
    }

    /// Casts the packet to `f32` the way librespot's `Converter::f64_to_f32` does, but into a
    /// buffer the queue hands back once it has played rather than a fresh one per packet.
    fn write(&mut self, packeted: AudioPacket, _converter: &mut Converter) -> SinkResult<()> {
        let samples = packeted
            .samples()
            .map_err(|error| SinkError::OnWrite(error.to_string()))?;
        let count = samples.len();
        let mut narrowed = self.paced.buffer(count);
        narrowed.extend(samples.iter().map(|sample| *sample as f32));
        let Some(samples) = packet(narrowed, NUM_CHANNELS as u16, SAMPLE_RATE) else {
            return Ok(());
        };
        if self
            .paced
            .write(samples)
            .and_then(|_| self.paced.drain())
            .is_err()
        {
            self.reopen();
            // the queue no longer paces the player, which would run the track out before the
            // engine retires it
            std::thread::sleep(lasts(count));
        }
        Ok(())
    }
}

/// Swallows the audio when no output device opens. It still answers the cue, so playback state
/// moves on rather than waiting for sound that cannot come, and it takes as long over a packet
/// as the packet lasts: without the queue to pace it, the player would otherwise run a track
/// out at decoding speed and be sitting on the end of the queue by the time a device is back.
struct Silence(Cue);

impl Sink for Silence {
    fn write(&mut self, packeted: AudioPacket, _converter: &mut Converter) -> SinkResult<()> {
        if let Ok(samples) = packeted.samples() {
            std::thread::sleep(lasts(samples.len()));
        }
        self.0.admit();
        Ok(())
    }
}

/// How long `samples` of librespot's audio plays for, counted across every channel.
fn lasts(samples: usize) -> Duration {
    Duration::from_secs_f64(samples as f64 / f64::from(NUM_CHANNELS as u32 * SAMPLE_RATE))
}
