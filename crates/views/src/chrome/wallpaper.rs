use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Hsla, ImageCacheError, ImageSource, ObjectFit, Pixels, RenderImage,
    Task, Window, div, img,
};
use state::{Outcome, Sonora, Toasts};
use ui::ActiveTheme as _;

use crate::root::Root;

/// The largest file read as a background image.
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// The image the listener laid behind the whole window. It is decoded once for the screen the
/// window is on and held here, apart from the artwork cache, whose budget a picture this size
/// would use up.
#[derive(Default)]
pub(crate) struct Wallpaper {
    /// The file `image` was read from, or the one being read now.
    path: Option<PathBuf>,
    image: Option<Arc<RenderImage>>,
    load: Option<Task<()>>,
}

impl Wallpaper {
    /// Starts reading the image the settings name when it is not the one held, and lets go of
    /// the held one when they name another or none. Returns whether the settings name an image,
    /// which is what makes the window opaque to the desktop, whether or not the file decodes.
    pub(crate) fn follow(&mut self, window: &mut Window, cx: &mut Context<Root>) -> bool {
        let wanted = Sonora::global(cx).settings.read(cx).background_image();
        if wanted == self.path.as_deref() {
            return self.path.is_some();
        }
        self.path = wanted.map(Path::to_path_buf);
        self.load = None;
        if let Some(image) = self.image.take() {
            cx.drop_image(image, Some(window));
        }
        let Some(path) = self.path.clone() else {
            return false;
        };

        let room = room(window, cx);
        let decode = cx.background_spawn({
            let path = path.clone();
            async move { read(&path, room) }
        });
        self.load = Some(cx.spawn(async move |root, cx| {
            let decoded = decode.await;
            root.update(cx, |root, cx| {
                if root.wallpaper.path.as_deref() != Some(path.as_path()) {
                    return;
                }
                match decoded {
                    Ok(image) => root.wallpaper.image = Some(Arc::new(image)),
                    Err(error) => {
                        log::warn!("wallpaper: cannot show {}: {error}", path.display());
                        Toasts::show(Outcome::Failed, "toast-background-image-failed", cx);
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        true
    }

    /// The layer under everything else the window draws. The page colour goes down at full
    /// strength, since nothing of the desktop shows through, then the image once it is decoded,
    /// then `veil`, the see-through page colour the root would otherwise have painted itself.
    /// `radius` rounds the layer where the window's corners are the app's to draw.
    pub(crate) fn layer(&self, radius: Option<Pixels>, veil: Option<Hsla>, cx: &App) -> AnyElement {
        let ground = Hsla {
            a: 1.,
            ..cx.theme().background
        };
        let rounded = |this: gpui::Div| match radius {
            Some(radius) => this.rounded(radius),
            None => this,
        };

        rounded(div().absolute().inset_0().bg(ground))
            .when_some(self.image.clone(), |this, image| {
                this.child(
                    img(ImageSource::Render(image))
                        .id("wallpaper")
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .when_some(radius, |this, radius| this.rounded(radius)),
                )
            })
            .when_some(veil, |this, veil| {
                this.child(rounded(div().absolute().inset_0().bg(veil)))
            })
            .into_any_element()
    }
}

/// The size in device pixels of the screen the window is on, which is the most a background is
/// ever drawn at. A window that cannot name its screen answers with its own size.
fn room(window: &Window, cx: &App) -> (u32, u32) {
    let size = window
        .display(cx)
        .map_or_else(|| window.viewport_size(), |display| display.bounds().size);
    let scale = window.scale_factor();
    let device = |side: Pixels| (side.as_f32() * scale).ceil().max(1.) as u32;

    (device(size.width), device(size.height))
}

/// Reads the file at `path` and decodes it for a screen of `room` device pixels.
fn read(path: &Path, room: (u32, u32)) -> Result<RenderImage, ImageCacheError> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_FILE_BYTES {
        return Err(ImageCacheError::Asset(
            format!("the file is {size} bytes, more than a background may be").into(),
        ));
    }
    ui::wallpaper(&std::fs::read(path)?, room)
}
