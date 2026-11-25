//! The sound path every engine shares: decoders, the progressive download they read, the
//! equalizer, the analysers, the paced queue and the output device. Nothing here knows a track,
//! a provider or a model, so it sits below `music`, which re-exports what the app reaches.
//!
//! It runs in real time, decoding, resampling and mixing every sample, so it is built fully
//! optimised even in debug builds. Code that is generic over a provider's types is compiled in
//! the provider's crate instead, at its optimisation level, which is why the decoders a provider
//! opens go through [`decode::Decoded`] rather than a type of its own.

pub mod aac;
pub mod audio;
pub mod decode;
pub mod equalizer;
pub mod mp4;
pub mod output;
pub mod ranged;
pub mod sink;
pub mod spectrum;
pub mod stream;
pub mod trim;
