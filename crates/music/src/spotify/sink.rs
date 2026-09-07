use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use librespot_playback::audio_backend::{Sink, SinkError, SinkResult};
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use tokio::sync::mpsc::UnboundedSender;

use crate::audio::{Output, Volume};
use crate::spectrum::Spectrum;

const QUEUED_CHUNKS: usize = 26;
const DRAIN_POLL: Duration = Duration::from_millis(10);
const DEVICE_POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Default)]
pub struct Flush(Arc<AtomicBool>);

impl Flush {
    pub fn request(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

pub struct BlazingSink {
    output: Output,
    flush: Flush,
    changed: UnboundedSender<()>,
    checked_at: Instant,
}

impl BlazingSink {
    pub fn open(
        flush: Flush,
        volume: Volume,
        spectrum: Spectrum,
        changed: UnboundedSender<()>,
    ) -> Result<Self, SinkError> {
        let output = Output::open(volume, spectrum)
            .map_err(|error| SinkError::ConnectionRefused(error.to_string()))?;
        output.sink().pause();

        Ok(Self {
            output,
            flush,
            changed,
            checked_at: Instant::now(),
        })
    }

    pub fn boxed(
        flush: Flush,
        volume: Volume,
        spectrum: Spectrum,
        changed: UnboundedSender<()>,
    ) -> Box<dyn Sink> {
        match Self::open(flush, volume, spectrum, changed) {
            Ok(sink) => Box::new(sink),
            Err(error) => {
                log::error!("sink: cannot open an output device: {error}");
                Box::new(Silence)
            }
        }
    }

    fn output_changed(&mut self) -> bool {
        let now = Instant::now();
        let changed = self.output.failed()
            || now.duration_since(self.checked_at) >= DEVICE_POLL && self.output.changed();
        if now.duration_since(self.checked_at) >= DEVICE_POLL {
            self.checked_at = now;
        }
        changed
    }

    fn disconnected(&self) -> SinkError {
        self.changed.send(()).ok();
        SinkError::OnWrite("audio output changed".to_owned())
    }
}

impl Sink for BlazingSink {
    fn start(&mut self) -> SinkResult<()> {
        if self.output.failed() || self.output.changed() {
            return Err(self.disconnected());
        }
        self.output.sink().play();
        Ok(())
    }

    fn stop(&mut self) -> SinkResult<()> {
        self.output.sink().pause();
        Ok(())
    }

    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        if self.output_changed() {
            return Err(self.disconnected());
        }

        if self.flush.take() {
            self.output.sink().clear();
            self.output.sink().play();
        }

        let samples = packet
            .samples()
            .map_err(|error| SinkError::OnWrite(error.to_string()))?;
        let samples = converter.f64_to_f32(samples);

        self.output.sink().append(rodio::buffer::SamplesBuffer::new(
            const { NonZero::new(NUM_CHANNELS as cpal::ChannelCount).unwrap() },
            const { NonZero::new(SAMPLE_RATE).unwrap() },
            &*samples,
        ));

        while self.output.sink().len() > QUEUED_CHUNKS {
            if self.output_changed() {
                return Err(self.disconnected());
            }
            std::thread::sleep(DRAIN_POLL);
        }
        Ok(())
    }
}

struct Silence;

impl Sink for Silence {
    fn write(&mut self, _packet: AudioPacket, _converter: &mut Converter) -> SinkResult<()> {
        Ok(())
    }
}
