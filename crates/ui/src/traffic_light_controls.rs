use gpui::prelude::*;
use gpui::{
    App, Div, Hsla, MouseButton, Pixels, StyleRefinement, Window, WindowControlArea, div, px, rgb,
    svg,
};

const SYSTEM_ACTS: bool = cfg!(target_os = "windows");
const DOT: Pixels = px(12.);
const GAP: Pixels = px(8.);
const GLYPH: Pixels = px(7.);
const GROUP: &str = "traffic-light-controls";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Close,
    Minimize,
    Maximize,
    Restore,
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

    fn id(self) -> &'static str {
        match self {
            Self::Minimize => "traffic-light-minimize",
            Self::Maximize | Self::Restore => "traffic-light-maximize",
            Self::Close => "traffic-light-close",
        }
    }

    fn dot(self) -> Hsla {
        match self {
            Self::Close => rgb(0xff5f57).into(),
            Self::Minimize => rgb(0xfebc2e).into(),
            Self::Maximize | Self::Restore => rgb(0x28c840).into(),
        }
    }

    fn system(self, window: &Window) -> bool {
        SYSTEM_ACTS && !window.is_fullscreen() && matches!(self, Self::Maximize | Self::Restore)
    }

    fn area(self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::Maximize | Self::Restore => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }
}

/// Traffic-light styled window controls: three plain colored dots that only reveal their
/// glyph on hover, in the familiar close/minimize/maximize order and coloring (not themed —
/// the whole point of this style is the recognizable, unthemed convention).
/// Position mirrors [`crate::WindowControls`]'s own `leading` flag: on the left the order
/// reads red, yellow, green; on the right it's mirrored so close stays at the outer edge.
#[derive(IntoElement)]
pub struct TrafficLightControls {
    base: Div,
    leading: bool,
}

impl TrafficLightControls {
    pub fn new(leading: bool) -> Self {
        Self {
            base: div(),
            leading,
        }
    }
}

impl Styled for TrafficLightControls {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for TrafficLightControls {
    fn render(mut self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let supported = window.window_controls();
        let fullscreen = window.is_fullscreen();
        let maximized = window.is_maximized() || fullscreen;
        let overrides = std::mem::take(self.base.style());
        let glyph_tint = Hsla {
            h: 0.,
            s: 0.,
            l: 0.,
            a: 0.45,
        };
        let glyph_hidden = Hsla {
            h: 0.,
            s: 0.,
            l: 0.,
            a: 0.,
        };

        let mut wanted: Vec<Control> = [
            Some(Control::Close),
            supported.minimize.then_some(Control::Minimize),
            supported.maximize.then_some(match maximized {
                true => Control::Restore,
                false => Control::Maximize,
            }),
        ]
        .into_iter()
        .flatten()
        .collect();

        if !self.leading {
            wanted.reverse();
        }

        let mut controls = self
            .base
            .group(GROUP)
            .flex()
            .flex_none()
            .items_center()
            .gap(GAP)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(wanted.into_iter().map(move |control| {
                div()
                    .id(control.id())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(DOT)
                    .rounded_full()
                    .bg(control.dot())
                    .cursor_pointer()
                    .occlude()
                    .window_control_area(control.area())
                    .child(
                        svg()
                            .path(icons::path(control.icon()))
                            .id("glyph")
                            .size(GLYPH)
                            .flex_none()
                            .text_color(glyph_hidden)
                            .group_hover(GROUP, move |style| style.text_color(glyph_tint)),
                    )
                    .when(!control.system(window), |this| {
                        this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                match control {
                                    Control::Minimize => window.minimize_window(),
                                    Control::Maximize | Control::Restore => {
                                        if window.is_fullscreen() {
                                            window.toggle_fullscreen();
                                        } else {
                                            window.zoom_window();
                                        }
                                    }
                                    Control::Close => window.remove_window(),
                                }
                            })
                    })
            }));
        controls.style().refine(&overrides);
        controls
    }
}
