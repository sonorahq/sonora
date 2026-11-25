//! The AAC decoder rodio is given in place of symphonia's own, so HE-AAC plays in full.
//!
//! Symphonia 0.5 decodes AAC-LC only. Handed an HE-AAC stream it reads past the SBR signalling
//! and plays the core alone, at half the rate and without the top half of the spectrum, which is
//! how the low streams of Tidal, YouTube Music and Apple Music arrive. A stream whose config
//! signals SBR goes to `rusty_aac` instead, which reconstructs it. Every other AAC stream stays
//! on symphonia's decoder, so nothing about AAC-LC playback changes.

use rusty_aac::AacDecoder as Extended;
use symphonia::core::audio::{AsAudioBufferRef as _, AudioBuffer, AudioBufferRef, Channels};
use symphonia::core::audio::{Signal as _, SignalSpec};
use symphonia::core::codecs::{
    CODEC_TYPE_AAC, CodecDescriptor, CodecParameters, Decoder, DecoderOptions, FinalizeResult,
};
use symphonia::core::errors::{Error, Result};
use symphonia::core::formats::Packet;
use symphonia::core::support_codec;
use symphonia::default::codecs::AacDecoder as Plain;

/// The frames one HE-AAC packet decodes to, twice the 1024 of its core.
const FRAMES: u64 = 2048;

/// An AAC decoder that sends HE-AAC to `rusty_aac` and everything else to symphonia. Register
/// it on a rodio builder with `with_symphonia_decoder`, which puts it ahead of symphonia's own.
pub struct Aac(Inner);

enum Inner {
    Plain(Box<Plain>),
    Extended(Box<Sbr>),
}

/// An HE-AAC stream decoding through `rusty_aac`, with the buffer each packet is copied into.
struct Sbr {
    decoder: Extended,
    /// The AudioSpecificConfig, kept to build a fresh decoder after a seek.
    config: Vec<u8>,
    params: CodecParameters,
    buffer: AudioBuffer<f32>,
}

impl Decoder for Aac {
    fn try_new(params: &CodecParameters, options: &DecoderOptions) -> Result<Self> {
        match Sbr::new(params) {
            Some(sbr) => Ok(Self(Inner::Extended(Box::new(sbr)))),
            None => {
                Plain::try_new(params, options).map(|plain| Self(Inner::Plain(Box::new(plain))))
            }
        }
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[support_codec!(
            CODEC_TYPE_AAC,
            "aac",
            "Advanced Audio Coding"
        )]
    }

    fn reset(&mut self) {
        match &mut self.0 {
            Inner::Plain(plain) => plain.reset(),
            Inner::Extended(sbr) => sbr.reset(),
        }
    }

    fn codec_params(&self) -> &CodecParameters {
        match &self.0 {
            Inner::Plain(plain) => plain.codec_params(),
            Inner::Extended(sbr) => &sbr.params,
        }
    }

    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        match &mut self.0 {
            Inner::Plain(plain) => plain.decode(packet),
            Inner::Extended(sbr) => sbr.decode(packet),
        }
    }

    fn finalize(&mut self) -> FinalizeResult {
        match &mut self.0 {
            Inner::Plain(plain) => plain.finalize(),
            Inner::Extended(_) => FinalizeResult::default(),
        }
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        match &self.0 {
            Inner::Plain(plain) => plain.last_decoded(),
            Inner::Extended(sbr) => sbr.buffer.as_audio_buffer_ref(),
        }
    }
}

impl Sbr {
    /// A decoder for `params` when its config signals SBR in a mono or stereo stream. `None`
    /// leaves the stream to symphonia, which is every AAC-LC stream and anything `rusty_aac`
    /// will not open.
    fn new(params: &CodecParameters) -> Option<Self> {
        if params.codec != CODEC_TYPE_AAC {
            return None;
        }
        let config = params.extra_data.as_deref()?.to_vec();
        let decoder = Extended::with_config_bytes(&config).ok()?;
        let sbr = decoder.sbr_config().filter(|sbr| sbr.sbr_present)?;
        // Parametric stereo widens a mono core into two channels.
        let count = match sbr.ps_present {
            true => 2,
            false => params.channels.map_or(2, Channels::count),
        };
        let channels = match count {
            1 => Channels::FRONT_LEFT,
            2 => Channels::FRONT_LEFT | Channels::FRONT_RIGHT,
            _ => return None,
        };
        log::debug!(
            "aac: decoding he-aac{} at {} Hz from a {} Hz core",
            match sbr.ps_present {
                true => " v2",
                false => "",
            },
            sbr.output_sample_rate,
            sbr.core_sample_rate
        );
        let spec = SignalSpec::new(sbr.output_sample_rate, channels);
        let mut params = params.clone();
        params
            .with_sample_rate(sbr.output_sample_rate)
            .with_channels(channels)
            .with_max_frames_per_packet(FRAMES);
        Some(Self {
            decoder,
            config,
            params,
            buffer: AudioBuffer::new(FRAMES, spec),
        })
    }

    /// Decodes one packet into the buffer, which `rusty_aac` hands back interleaved and
    /// symphonia wants planar. A packet that fails leaves an empty buffer and a decode error,
    /// which rodio skips past.
    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        self.buffer.clear();
        let frame = match self.decoder.decode(packet.buf(), None) {
            Ok(frame) => frame,
            Err(rusty_aac::Error::Again) => return Ok(self.buffer.as_audio_buffer_ref()),
            Err(error) => {
                log::debug!("aac: cannot decode a packet: {error}");
                return Err(Error::DecodeError("aac: cannot decode the packet"));
            }
        };
        let channels = self.buffer.spec().channels.count();
        if usize::from(frame.channels) != channels {
            return Err(Error::DecodeError(
                "aac: the packet changed its channel count",
            ));
        }
        let frames = frame.frames();
        if frames as u64 > self.buffer.capacity() as u64 {
            self.buffer = AudioBuffer::new(frames as u64, *self.buffer.spec());
        }
        self.buffer.render_reserved(Some(frames));
        for channel in 0..channels {
            let plane = self.buffer.chan_mut(channel);
            for (at, sample) in plane.iter_mut().enumerate() {
                *sample = frame.samples[at * channels + channel];
            }
        }
        Ok(self.buffer.as_audio_buffer_ref())
    }

    /// Starts over with a fresh decoder, as after a seek, so no state of the old position
    /// bleeds into the new one.
    fn reset(&mut self) {
        if let Ok(decoder) = Extended::with_config_bytes(&self.config) {
            self.decoder = decoder;
        }
    }
}
