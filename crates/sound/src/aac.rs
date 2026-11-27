//! The AAC decoder rodio is given in place of symphonia's own, so HE-AAC plays in full.
//!
//! Symphonia decodes AAC-LC only. Handed an HE-AAC stream it reads past the SBR signalling and
//! plays the core alone, at half the rate and without the top half of the spectrum, which is how
//! the low streams of Tidal, YouTube Music and Apple Music arrive. A stream whose config signals
//! SBR goes to `rusty_aac` instead, which reconstructs it. Every other AAC stream gets
//! symphonia's own decoder, so nothing about AAC-LC playback changes.

use rusty_aac::AacDecoder as Extended;
use symphonia::core::audio::{AsGenericAudioBufferRef as _, Audio as _, AudioMut as _};
use symphonia::core::audio::{AudioBuffer, AudioSpec, Channels, GenericAudioBufferRef, Position};
use symphonia::core::codecs::CodecInfo;
use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia::core::codecs::audio::{
    AudioCodecParameters, AudioDecoder, AudioDecoderOptions, FinalizeResult,
};
use symphonia::core::codecs::registry::{RegisterableAudioDecoder, SupportedAudioCodec};
use symphonia::core::errors::{Error, Result};
use symphonia::core::packet::PacketRef;
use symphonia::default::codecs::AacDecoder as Plain;

/// The frames one HE-AAC packet decodes to, twice the 1024 of its core.
const FRAMES: usize = 2048;

/// The one codec this decoder takes over from symphonia's own.
const SUPPORTED: [SupportedAudioCodec; 1] = [SupportedAudioCodec {
    id: CODEC_ID_AAC,
    info: CodecInfo {
        short_name: "aac",
        long_name: "Advanced Audio Coding",
        profiles: &[],
    },
}];

/// An HE-AAC stream decoding through `rusty_aac`, with the buffer each packet is copied into.
/// Register it on a rodio builder with `with_symphonia_decoder`, which replaces symphonia's own
/// AAC decoder. A stream without SBR is handed straight back to that decoder.
pub struct Aac {
    decoder: Extended,
    /// The AudioSpecificConfig, kept to build a fresh decoder after a seek.
    config: Vec<u8>,
    params: AudioCodecParameters,
    buffer: AudioBuffer<f32>,
}

impl AudioDecoder for Aac {
    /// Starts over with a fresh decoder, as after a seek, so no state of the old position
    /// bleeds into the new one.
    fn reset(&mut self) {
        if let Ok(decoder) = Extended::with_config_bytes(&self.config) {
            self.decoder = decoder;
        }
    }

    fn codec_info(&self) -> &CodecInfo {
        &SUPPORTED[0].info
    }

    fn codec_params(&self) -> &AudioCodecParameters {
        &self.params
    }

    /// Decodes one packet into the buffer, which `rusty_aac` hands back interleaved and
    /// symphonia wants planar. A packet that fails leaves an empty buffer and a decode error,
    /// which rodio skips past.
    fn decode_ref(&mut self, packet: &PacketRef<'_>) -> Result<GenericAudioBufferRef<'_>> {
        self.buffer.clear();
        let frame = match self.decoder.decode(packet.data, None) {
            Ok(frame) => frame,
            Err(rusty_aac::Error::Again) => return Ok(self.buffer.as_generic_audio_buffer_ref()),
            Err(error) => {
                log::debug!("aac: cannot decode a packet: {error}");
                return Err(Error::DecodeError("aac: cannot decode the packet"));
            }
        };
        let channels = self.buffer.spec().channels().count();
        if usize::from(frame.channels) != channels {
            return Err(Error::DecodeError(
                "aac: the packet changed its channel count",
            ));
        }
        let frames = frame.frames();
        if frames > self.buffer.capacity() {
            self.buffer.grow_capacity(frames);
        }
        self.buffer.render_uninit(Some(frames));
        let samples = &frame.samples[..frames * channels];
        self.buffer.copy_from_slice_interleaved(&samples);
        Ok(self.buffer.as_generic_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult::default()
    }

    fn last_decoded(&self) -> GenericAudioBufferRef<'_> {
        self.buffer.as_generic_audio_buffer_ref()
    }
}

impl RegisterableAudioDecoder for Aac {
    fn try_registry_new(
        params: &AudioCodecParameters,
        options: &AudioDecoderOptions,
    ) -> Result<Box<dyn AudioDecoder>> {
        match Aac::new(params) {
            Some(aac) => Ok(Box::new(aac)),
            None => Plain::try_registry_new(params, options),
        }
    }

    fn supported_codecs() -> &'static [SupportedAudioCodec] {
        &SUPPORTED
    }
}

impl Aac {
    /// A decoder for `params` when its config signals SBR in a mono or stereo stream. `None`
    /// leaves the stream to symphonia, which is every AAC-LC stream and anything `rusty_aac`
    /// will not open.
    fn new(params: &AudioCodecParameters) -> Option<Self> {
        if params.codec != CODEC_ID_AAC {
            return None;
        }
        let config = params.extra_data.as_deref()?.to_vec();
        let decoder = Extended::with_config_bytes(&config).ok()?;
        let sbr = decoder.sbr_config().filter(|sbr| sbr.sbr_present)?;
        // Parametric stereo widens a mono core into two channels.
        let count = match sbr.ps_present {
            true => 2,
            false => params.channels.as_ref().map_or(2, Channels::count),
        };
        let channels = match count {
            1 => Channels::Positioned(Position::FRONT_LEFT),
            2 => Channels::Positioned(Position::FRONT_LEFT | Position::FRONT_RIGHT),
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
        let spec = AudioSpec::new(sbr.output_sample_rate, channels.clone());
        let mut params = params.clone();
        params
            .with_sample_rate(sbr.output_sample_rate)
            .with_channels(channels)
            .with_max_frames_per_packet(FRAMES as u64);
        Some(Self {
            decoder,
            config,
            params,
            buffer: AudioBuffer::new(spec, FRAMES),
        })
    }
}
