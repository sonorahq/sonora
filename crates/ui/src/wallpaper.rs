use std::io::Cursor;

use gpui::{App, Global, ImageCacheError, RenderImage};
use image::{
    AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageFormat, RgbaImage,
    codecs::{gif::GifDecoder, webp::WebPDecoder},
    imageops,
};

use crate::artwork::bgra;

/// The transparency a window takes when an image is first laid behind it: all of it, so the
/// picture shows at full strength and the listener takes it down from there.
pub const WALLPAPER_TRANSPARENCY: f32 = 1.;
/// The longest side a window background is held at, which a 4K screen fits inside and every
/// GPU takes as one texture.
const MAX_EDGE: u32 = 4096;
/// What the frames of an animated background may take together. Every frame is held decoded
/// and uploaded to the GPU once shown, so a long animation is kept at a lower resolution.
const ANIMATION_BYTES: usize = 128 * 1024 * 1024;

/// Whether an image is laid behind the window, which is then opaque to the desktop. A surface
/// that thins its own alpha to let the desktop through has nothing behind it there and would
/// come out dark, so it asks here and lets the image through instead. The root keeps it current.
#[derive(Clone, Copy)]
pub struct Backed(bool);

impl Global for Backed {}

impl Backed {
    pub fn get(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|backed| backed.0)
    }

    /// Records whether the window has an image behind it, touching the global only when the
    /// answer changed so its observers run once per change.
    pub fn set(backed: bool, cx: &mut App) {
        if !cx.has_global::<Self>() || Self::get(cx) != backed {
            cx.set_global(Self(backed));
        }
    }
}

/// Decodes the image in `bytes` to lay behind a window on a screen of `room` device pixels.
///
/// The image is scaled down until it just covers the screen, so it is never held larger than
/// it can be drawn. An animated GIF or WebP keeps all of its frames, halved in size as often as
/// it takes to fit them in `ANIMATION_BYTES`.
pub fn wallpaper(bytes: &[u8], room: (u32, u32)) -> Result<RenderImage, ImageCacheError> {
    let started = std::time::Instant::now();
    let room = (room.0.clamp(1, MAX_EDGE), room.1.clamp(1, MAX_EDGE));
    let format = image::guess_format(bytes)?;
    let frames = match format {
        ImageFormat::Gif => animation(GifDecoder::new(Cursor::new(bytes))?, room)?,
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes))?;
            match decoder.has_animation() {
                true => {
                    let _ = decoder.set_background_color(image::Rgba([0, 0, 0, 0]));
                    animation(decoder, room)?
                }
                false => vec![still(decoder, room)?],
            }
        }
        _ => {
            let decoder =
                image::ImageReader::with_format(Cursor::new(bytes), format).into_decoder()?;
            vec![still(decoder, room)?]
        }
    };
    let image = RenderImage::new(frames);
    let held = image.size(0);
    log::debug!(
        "wallpaper: {format:?} of {} bytes held at {}x{} in {} frames for a {}x{} screen, {:?}",
        bytes.len(),
        held.width.0,
        held.height.0,
        image.frame_count(),
        room.0,
        room.1,
        started.elapsed()
    );
    Ok(image)
}

fn still(mut decoder: impl ImageDecoder, room: (u32, u32)) -> Result<Frame, ImageCacheError> {
    let orientation = decoder.orientation()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let size = covering((image.width(), image.height()), room);
    let mut image = scaled(image, size);
    bgra(&mut image);
    Ok(Frame::new(image))
}

/// Decodes every frame of an animation at the size that covers `room`, halving all of them
/// each time they outgrow `ANIMATION_BYTES`. A frame that does not decode is left out.
fn animation<'a>(
    decoder: impl AnimationDecoder<'a>,
    room: (u32, u32),
) -> Result<Vec<Frame>, ImageCacheError> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut size = None;
    let mut spent = 0_usize;

    for frame in decoder.into_frames() {
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => {
                log::debug!("wallpaper: skipping an animation frame: {error}");
                continue;
            }
        };
        let delay = frame.delay();
        let image = DynamicImage::ImageRgba8(frame.into_buffer());
        let (width, height) =
            *size.get_or_insert_with(|| covering((image.width(), image.height()), room));
        let image = scaled(image, (width, height));
        spent = spent.saturating_add(image.as_raw().len());
        frames.push(Frame::from_parts(image, 0, 0, delay));

        if spent > ANIMATION_BYTES && width > 1 && height > 1 {
            let half = (width / 2, height / 2);
            for frame in &mut frames {
                let small = imageops::thumbnail(frame.buffer(), half.0, half.1);
                *frame = Frame::from_parts(small, 0, 0, frame.delay());
            }
            spent = frames
                .iter()
                .map(|frame| frame.buffer().as_raw().len())
                .sum();
            size = Some(half);
        }
    }
    if frames.is_empty() {
        return Err(ImageCacheError::Asset(
            "the animation holds no frame that decodes".into(),
        ));
    }
    for frame in &mut frames {
        bgra(frame.buffer_mut());
    }
    Ok(frames)
}

/// The size an image of `size` is held at to cover `room`: scaled down, keeping its aspect,
/// until one side meets the room's and the other still reaches past it. An image that is
/// already no larger keeps its size. A side that would still be longer than `MAX_EDGE`, as a
/// panorama's is, brings the whole image down to it, since no texture may be larger.
fn covering(size: (u32, u32), room: (u32, u32)) -> (u32, u32) {
    let (width, height) = (size.0.max(1) as f32, size.1.max(1) as f32);
    let cover = (room.0 as f32 / width).max(room.1 as f32 / height);
    let fit = MAX_EDGE as f32 / width.max(height);
    let scale = cover.min(fit);
    match scale < 1. {
        true => (
            ((width * scale).round() as u32).max(1),
            ((height * scale).round() as u32).max(1),
        ),
        false => (size.0.max(1), size.1.max(1)),
    }
}

/// `image` as RGBA at exactly `size`. More than halving an image goes through the cheaper
/// box filter, which is also the one that does not alias at that ratio.
fn scaled(image: DynamicImage, size: (u32, u32)) -> RgbaImage {
    let image = match (image.width(), image.height()) {
        held if held == size => image,
        (width, _) if width > size.0.saturating_mul(2) => image.thumbnail_exact(size.0, size.1),
        _ => image.resize_exact(size.0, size.1, imageops::FilterType::Triangle),
    };
    image.into_rgba8()
}
