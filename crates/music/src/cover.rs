//! Turns a picture the listener picked into a playlist cover a provider accepts.

use std::fmt;
use std::io::{Cursor, Read as _};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageReader};

/// The largest picture Sonora reads before cropping, so a huge photo fails quickly instead of
/// holding hundreds of megabytes while it decodes.
const MAX_SOURCE: u64 = 40 * 1024 * 1024;
/// How many bytes of a file are enough to tell what kind of picture it is.
const SNIFF: u64 = 64;
/// The JPEG qualities tried in turn until the cover fits a provider's byte limit.
const QUALITIES: [u8; 6] = [90, 82, 74, 66, 58, 50];

/// Why a file cannot become a cover. It travels in the `anyhow` chain, so a caller can tell
/// the listener's file apart from a provider that refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unusable {
    /// The file could not be opened or read.
    Unopenable,
    /// The file is larger than Sonora reads.
    TooLarge,
    /// The file is not a JPEG, PNG or WebP picture, or one too damaged to decode.
    NotAPicture,
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unopenable => "the file cannot be read",
            Self::TooLarge => "the picture is too large",
            Self::NotAPicture => "the file is not a picture Sonora can read",
        })
    }
}

impl std::error::Error for Unusable {}

/// The [`Unusable`] somewhere in `error`'s chain, if the error is about the file.
pub fn unusable(error: &anyhow::Error) -> Option<Unusable> {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref().copied())
}

/// Checks a file by its size and first bytes, without decoding it, so a wrong pick is
/// refused at once.
pub fn check_file(path: &Path) -> Result<(), Unusable> {
    let size = std::fs::metadata(path)
        .map_err(|_| Unusable::Unopenable)?
        .len();
    if size > MAX_SOURCE {
        return Err(Unusable::TooLarge);
    }
    let mut head = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(SNIFF).read_to_end(&mut head))
        .map_err(|_| Unusable::Unopenable)?;
    check(&head)
}

/// Whether `bytes` start like a picture [`square_jpeg`] can decode.
fn check(bytes: &[u8]) -> Result<(), Unusable> {
    let format = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|reader| reader.format());
    match format {
        Some(format) if format.reading_enabled() => Ok(()),
        _ => Err(Unusable::NotAPicture),
    }
}

/// What a provider takes as a playlist cover: a square JPEG at most `side` pixels wide and at
/// most `bytes` long.
#[derive(Clone, Copy, Debug)]
pub struct CoverLimits {
    pub side: u32,
    pub bytes: usize,
}

/// Crops `source` to its centre square, scales it down to `limits.side` and encodes it as a
/// baseline JPEG small enough for `limits.bytes`. A picture smaller than the side keeps its
/// own size. Fails with [`Unusable`] when the bytes are not a picture Sonora can decode, and
/// otherwise when no quality fits.
pub fn square_jpeg(source: &[u8], limits: CoverLimits) -> Result<Vec<u8>> {
    if source.len() as u64 > MAX_SOURCE {
        return Err(Unusable::TooLarge.into());
    }
    check(source)?;
    let decoded = ImageReader::new(Cursor::new(source))
        .with_guessed_format()
        .map_err(|_| Unusable::NotAPicture)?
        .decode()
        .map_err(|_| Unusable::NotAPicture)?;
    let square = squared(decoded, limits.side);

    for quality in QUALITIES {
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, quality)
            .encode_image(&square)
            .context("cannot encode the cover")?;
        if out.len() <= limits.bytes {
            return Ok(out);
        }
    }
    bail!("the cover does not fit in {} bytes", limits.bytes)
}

fn squared(image: DynamicImage, side: u32) -> DynamicImage {
    let edge = image.width().min(image.height());
    let left = (image.width() - edge) / 2;
    let top = (image.height() - edge) / 2;
    let cropped = image.crop_imm(left, top, edge, edge);
    let rgb = DynamicImage::ImageRgb8(cropped.to_rgb8());
    match edge > side {
        true => rgb.resize_exact(side, side, FilterType::Lanczos3),
        false => rgb,
    }
}
