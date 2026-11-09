//! Tooltips. Rune shows an element's `.help(text)` after the hover delay and hides it on a press,
//! a scroll or the pointer leaving; this module gives those tooltips Sonora's look and the old
//! `tip` vocabulary.

use rune::{El, HelpLook, Shadow, TextStyle};

use crate::label::Weight;
use crate::metrics::Text;
use crate::theme::Theme;

/// The bubble's padding, across and down.
const PAD_X: f32 = 8.;
const PAD_Y: f32 = 4.;
const BORDER: f32 = 1.;
/// How far the bubble stands off its element, and how far it keeps inside the window.
const OFFSET: f32 = 6.;
const MARGIN: f32 = 8.;
/// How long the pointer rests before the bubble shows, and how soon after one hides the next
/// shows at once.
const DELAY: f64 = 0.5;
const SKIP: f64 = 0.3;
/// The bubble's entrance and exit, in seconds, and the scale it grows from.
const ENTER: f64 = 0.12;
const EXIT: f64 = 0.08;
const GROW: f32 = 0.96;

/// Where a tooltip sits against its element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Perch {
    /// Centred below the element, or above it when the window has no room below.
    #[default]
    Below,
    /// Centred above the element, or below it when the window has no room above.
    Above,
    /// Was: following the pointer. Rune's tooltips sit against their element, so this reads as
    /// [`Perch::Below`].
    Follow,
}

/// Attaches a tooltip to an element.
pub trait Tipped: Sized {
    /// Shows the Fluent message `key` on hover. Call it while building the view, where the
    /// message resolves in the current language.
    fn tip(self, key: &str, perch: Perch) -> Self;
    /// Shows `text` on hover as it is.
    fn tip_label(self, text: impl Into<String>, perch: Perch) -> Self;
}

impl Tipped for El {
    fn tip(self, key: &str, perch: Perch) -> Self {
        self.tip_label(i18n::lookup(key, None).into_owned(), perch)
    }

    fn tip_label(self, text: impl Into<String>, perch: Perch) -> Self {
        let tipped = self.help(text);
        match perch {
            Perch::Above => tipped.help_above(),
            Perch::Below | Perch::Follow => tipped,
        }
    }
}

/// How Rune draws Sonora's tooltips where the app draws them (Linux, Windows, overlays): the
/// popover fill with a hairline border, in the small step. The view sets it with
/// `cx.help_look(ui::help_look(&theme))` each time it runs, so it follows the theme.
pub fn help_look(theme: &Theme) -> HelpLook {
    let dark = theme.background.l < 0.5;
    HelpLook::system(dark)
        .fill(theme.popover.color())
        .ink(theme.popover_foreground.color())
        .ring(BORDER, theme.border.color())
        .radius(theme.radius)
        .padding(PAD_X, PAD_Y)
        .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
        .shadows([Shadow {
            y: 1.,
            blur: 2.,
            color: rune::Color::BLACK.alpha(0.05),
            ..Shadow::default()
        }])
        .gap(OFFSET)
        .margin(MARGIN)
        .delay(DELAY)
        .skip_delay(SKIP)
        .fade(ENTER, EXIT, GROW)
}
