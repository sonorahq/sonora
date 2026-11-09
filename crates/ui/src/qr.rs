//! A QR code for some text, drawn black on white at a fixed size, quiet zone included.

use qrcodegen::{QrCode, QrCodeEcc};
use rune::{Color, El, Shape, canvas, el};

/// Modules of blank border the QR standard asks for on every side, which a camera needs to find
/// where the code starts.
const QUIET: i32 = 4;
/// A code has to be dark on light to scan in every camera, whatever the theme is, so its two
/// colours are literals and never theme tokens.
const DARK: Color = Color::BLACK;
const LIGHT: Color = Color::WHITE;

/// The code, painted from the module grid as one filled run of dark modules per stretch of a
/// row. Text too long for a code draws the white square alone.
pub struct Qr {
    text: String,
    size: f32,
    radius: f32,
    /// What a screen reader calls the code, such as what scanning it does.
    name: Option<String>,
}

impl Qr {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            size: 0.,
            radius: 0.,
            name: None,
        }
    }

    /// Rounds the white square's corners. The quiet zone keeps the modules clear of the curve
    /// for any radius up to a few modules.
    pub fn rounded(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// The side of the square, quiet zone included.
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Reads the code to screen readers as an image named `name`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn build(self) -> El {
        let square = el().size(self.size, self.size).fill(LIGHT);
        let Ok(code) = QrCode::encode_text(&self.text, QrCodeEcc::Medium) else {
            return square.radius(self.radius).a11y_hidden();
        };
        let side = code.size() + QUIET * 2;
        let module = self.size / side as f32;
        let mut shape = Shape::new().view_box(side as f32, side as f32);
        for y in 0..code.size() {
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
                let (left, right) = ((start + QUIET) as f32, (x + QUIET) as f32);
                let (top, bottom) = ((y + QUIET) as f32, (y + QUIET + 1) as f32);
                shape = shape.fill(
                    vec![[left, top], [right, top], [right, bottom], [left, bottom]],
                    DARK,
                );
            }
        }
        let code = canvas(shape)
            .size(self.size, self.size)
            .fill(LIGHT)
            .radius(self.radius.min(module * 3.));
        match self.name {
            Some(name) => code.role("image").label(name),
            None => code.a11y_hidden(),
        }
    }
}
