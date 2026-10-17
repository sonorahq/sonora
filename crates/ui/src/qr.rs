use gpui::prelude::*;
use gpui::{App, Bounds, Hsla, Pixels, SharedString, Window, canvas, fill, point};
use qrcodegen::{QrCode, QrCodeEcc};

use crate::snapped;

/// Modules of blank border the QR standard asks for on every side, which a camera needs to find
/// where the code starts.
const QUIET: i32 = 4;
/// The colour of a dark module. A code has to be dark on light to scan in every camera, whatever
/// the theme is, so this is a literal and never a theme token.
const DARK: Hsla = gpui::black();
/// The colour of the square behind the modules and its quiet zone, a literal for the same reason
/// as `DARK`.
const LIGHT: Hsla = gpui::white();

/// A QR code for some text, drawn black on white at a fixed size, quiet zone included. It is
/// built while a frame paints and dropped when the frame is done, and it is painted from the
/// module grid as one quad per run of dark modules, so it holds no image and caches nothing.
/// Text too long for a code draws nothing.
#[derive(IntoElement)]
pub struct Qr {
    text: SharedString,
    size: Pixels,
}

impl Qr {
    /// A code for `text`, as wide and tall as `size` once that is set.
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            size: Pixels::ZERO,
        }
    }

    /// The side of the square, quiet zone included.
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for Qr {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self { text, size } = self;

        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| paint(&text, bounds, window),
        )
        .flex_none()
        .w(size)
        .h(size)
    }
}

/// Encodes `text` and paints it into the square at the top left of `bounds`. Each module edge is
/// snapped to a whole device pixel, so a run never leaves a seam against the row beside it.
fn paint(text: &str, bounds: Bounds<Pixels>, window: &mut Window) {
    let Ok(code) = QrCode::encode_text(text, QrCodeEcc::Medium) else {
        return;
    };
    let side = bounds.size.width.min(bounds.size.height);
    let module = side / (code.size() + QUIET * 2) as f32;
    let origin = bounds.origin;
    window.paint_quad(fill(Bounds::new(origin, gpui::size(side, side)), LIGHT));

    for y in 0..code.size() {
        let top = edge(origin.y, module, y, window);
        let bottom = edge(origin.y, module, y + 1, window);
        let mut x = 0;
        while x < code.size() {
            if !code.get_module(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < code.size() && code.get_module(x, y) {
                x += 1;
            }
            let left = edge(origin.x, module, start, window);
            let right = edge(origin.x, module, x, window);
            window.paint_quad(fill(
                Bounds::from_corners(point(left, top), point(right, bottom)),
                DARK,
            ));
        }
    }
}

/// Where the line before module `at` falls along an axis that starts at `origin`, snapped to a
/// device pixel.
fn edge(origin: Pixels, module: Pixels, at: i32, window: &Window) -> Pixels {
    snapped(origin + module * (QUIET + at) as f32, window)
}
