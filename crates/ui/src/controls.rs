//! Window controls the app draws itself, for a window whose title bar it draws: themed
//! minimize, maximize and close buttons, or the three coloured dots of the macOS convention.

use rune::{Cursor, El, Tx, icon, row};

use crate::color::{Hsla, rgb};
use crate::motion::PRESS_SCALE;
use crate::theme::Theme;

/// The themed buttons' side and their glyphs' size.
const BUTTON: f32 = 20.;
const GLYPH: f32 = 16.;
const GAP: f32 = 8.;
/// The dots' size, the space between them and their glyphs' size.
const DOT: f32 = 12.;
const DOT_GAP: f32 = 8.;
const DOT_GLYPH: f32 = 7.;
/// The ink of a dot's glyph, which only shows while the pointer is over the dots.
const DOT_INK: Hsla = Hsla {
    h: 0.,
    s: 0.,
    l: 0.,
    a: 0.45,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl Control {
    fn icon(self) -> &'static str {
        match self {
            Self::Minimize => "icons/window-minimize.svg",
            Self::Maximize => "icons/window-maximize.svg",
            Self::Restore => "icons/window-restore.svg",
            Self::Close => "icons/window-close.svg",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Minimize => "window-minimize",
            Self::Maximize | Self::Restore => "window-maximize",
            Self::Close => "window-close",
        }
    }

    /// What a screen reader calls the control.
    fn name(self) -> String {
        match self {
            Self::Minimize => i18n::t!("app-minimize"),
            Self::Maximize => i18n::t!("window-maximize"),
            Self::Restore => i18n::t!("window-restore"),
            Self::Close => i18n::t!("app-close-window"),
        }
        .into_owned()
    }

    /// The traffic light's colour: not themed, since the convention is the point.
    fn dot(self) -> Hsla {
        match self {
            Self::Close => rgb(0xff5f57).into(),
            Self::Minimize => rgb(0xfebc2e).into(),
            Self::Maximize | Self::Restore => rgb(0x28c840).into(),
        }
    }

    fn act(self, tx: &mut Tx) {
        match self {
            Self::Minimize => tx.minimize(),
            Self::Maximize | Self::Restore => tx.toggle_maximized(),
            Self::Close => tx.close_window(),
        }
    }
}

/// The controls this window offers, in order from the outer edge inward.
fn wanted(maximized: bool, minimize: bool, maximize: bool) -> Vec<Control> {
    [
        minimize.then_some(Control::Minimize),
        maximize.then_some(match maximized {
            true => Control::Restore,
            false => Control::Maximize,
        }),
        Some(Control::Close),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Minimize, maximize and close as small themed buttons. Close lights in the danger colour.
/// Rune does not say whether the window is maximized, so the owner passes it from what it last
/// asked for.
pub struct WindowControls {
    leading: bool,
    maximized: bool,
    minimize: bool,
    maximize: bool,
}

impl WindowControls {
    /// `leading` puts the controls at the window's start edge, close outermost.
    pub fn new(leading: bool) -> Self {
        Self {
            leading,
            maximized: false,
            minimize: true,
            maximize: true,
        }
    }

    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    /// Leaves out minimize, maximize or both, for a window that offers neither.
    pub fn offers(mut self, minimize: bool, maximize: bool) -> Self {
        self.minimize = minimize;
        self.maximize = maximize;
        self
    }

    pub fn build<S: 'static>(self, theme: &Theme) -> El {
        let mut controls = wanted(self.maximized, self.minimize, self.maximize);
        if self.leading {
            controls.reverse();
        }
        row()
            .key("window-controls")
            .center_y()
            .gap(GAP)
            .no_window_drag()
            .children(controls.into_iter().map(|control| {
                let danger = control == Control::Close;
                let (hover, ink) = match danger {
                    true => (theme.danger, theme.danger_foreground),
                    false => (theme.secondary_active, theme.foreground),
                };
                let name = control.name();
                row()
                    .key(control.key())
                    .role("button")
                    .label(name.clone())
                    .help(name)
                    .center()
                    .size(BUTTON, BUTTON)
                    .radius(theme.radius)
                    .hover(hover.color())
                    .focusable(true)
                    .cursor(Cursor::Pointer)
                    .press_scale(PRESS_SCALE)
                    .child(
                        icon(icons::get(control.icon()), GLYPH)
                            .color(theme.muted_foreground.color())
                            .group_color(ink.color())
                            .a11y_hidden(),
                    )
                    .on_press_tx(move |_: &mut S, tx: &mut Tx| control.act(tx))
            }))
    }
}

/// The three coloured dots, which show their glyphs only while the pointer is over them. On the
/// start edge they read red, yellow, green; on the end edge the order mirrors so close stays
/// outermost.
pub struct TrafficLightControls {
    leading: bool,
    maximized: bool,
    minimize: bool,
    maximize: bool,
}

impl TrafficLightControls {
    pub fn new(leading: bool) -> Self {
        Self {
            leading,
            maximized: false,
            minimize: true,
            maximize: true,
        }
    }

    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    pub fn offers(mut self, minimize: bool, maximize: bool) -> Self {
        self.minimize = minimize;
        self.maximize = maximize;
        self
    }

    pub fn build<S: 'static>(self, _theme: &Theme) -> El {
        let mut controls = wanted(self.maximized, self.minimize, self.maximize);
        // close, minimize, maximize from the start edge
        controls.rotate_right(1);
        if !self.leading {
            controls.reverse();
        }
        row()
            .key("traffic-lights")
            .hover_group()
            .center_y()
            .gap(DOT_GAP)
            .no_window_drag()
            .children(controls.into_iter().map(|control| {
                let name = control.name();
                row()
                    .key(control.key())
                    .role("button")
                    .label(name.clone())
                    .help(name)
                    .center()
                    .size(DOT, DOT)
                    .radius(DOT / 2.)
                    .fill(control.dot().color())
                    .focusable(true)
                    .cursor(Cursor::Pointer)
                    .press_scale(PRESS_SCALE)
                    .child(
                        icon(icons::get(control.icon()), DOT_GLYPH)
                            .color(DOT_INK.color())
                            .group_opacity(0., 1.)
                            .a11y_hidden(),
                    )
                    .on_press_tx(move |_: &mut S, tx: &mut Tx| control.act(tx))
            }))
    }
}
