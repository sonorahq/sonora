use std::sync::OnceLock;

use gpui::prelude::*;
use gpui::{
    App, Div, Font, MouseButton, Pixels, SharedString, StyleRefinement, Window, WindowControlArea,
    div, font, px, svg,
};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui::{CursorStyle, Decorations, ResizeEdge};

use i18n::t;

use crate::theme::ActiveTheme as _;

const SYSTEM_ACTS: bool = cfg!(target_os = "windows");
const BUTTON: Pixels = px(20.);
const GLYPH: Pixels = px(16.);
const CAPTION_GLYPH: Pixels = px(10.);
const CAPTION_NUDGE: Pixels = px(1.);
/// The icon fonts that carry Windows' caption glyphs, newest first. Windows 11 ships both, and
/// Windows 10 only the second.
const CAPTION_FONTS: [&str; 2] = ["Segoe Fluent Icons", "Segoe MDL2 Assets"];
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const RESIZE_EDGE: Pixels = px(5.);
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const RESIZE_CORNER: Pixels = px(10.);

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

    fn caption_glyph(self) -> &'static str {
        match self {
            Self::Minimize => "\u{E921}",
            Self::Maximize => "\u{E922}",
            Self::Restore => "\u{E923}",
            Self::Close => "\u{E8BB}",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Minimize => "window-minimize",
            Self::Maximize | Self::Restore => "window-maximize",
            Self::Close => "window-close",
        }
    }

    /// What a screen reader calls the control.
    fn name(self) -> SharedString {
        match self {
            Self::Minimize => t!("app-minimize"),
            Self::Maximize => t!("window-maximize"),
            Self::Restore => t!("window-restore"),
            Self::Close => t!("app-close-window"),
        }
    }

    fn system(self) -> bool {
        SYSTEM_ACTS && matches!(self, Self::Maximize | Self::Restore)
    }

    fn area(self) -> WindowControlArea {
        match self {
            Self::Minimize => WindowControlArea::Min,
            Self::Maximize | Self::Restore => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[derive(IntoElement)]
pub struct WindowFrame {
    base: Div,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl WindowFrame {
    #[track_caller]
    pub fn new() -> Self {
        Self { base: div() }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl Default for WindowFrame {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl Styled for WindowFrame {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl RenderOnce for WindowFrame {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self { mut base } = self;
        let overrides = std::mem::take(base.style());
        let tiling = match window.window_decorations() {
            Decorations::Client { tiling } if !window.is_maximized() => Some(tiling),
            _ => None,
        };

        let mut frame = base.absolute().inset_0().when_some(tiling, |this, tiling| {
            this.when(!tiling.top, |this| {
                this.child(resize_handle(ResizeEdge::Top))
            })
            .when(!tiling.bottom, |this| {
                this.child(resize_handle(ResizeEdge::Bottom))
            })
            .when(!tiling.left, |this| {
                this.child(resize_handle(ResizeEdge::Left))
            })
            .when(!tiling.right, |this| {
                this.child(resize_handle(ResizeEdge::Right))
            })
            .when(!tiling.top && !tiling.left, |this| {
                this.child(resize_handle(ResizeEdge::TopLeft))
            })
            .when(!tiling.top && !tiling.right, |this| {
                this.child(resize_handle(ResizeEdge::TopRight))
            })
            .when(!tiling.bottom && !tiling.left, |this| {
                this.child(resize_handle(ResizeEdge::BottomLeft))
            })
            .when(!tiling.bottom && !tiling.right, |this| {
                this.child(resize_handle(ResizeEdge::BottomRight))
            })
        });
        frame.style().refine(&overrides);
        frame
    }
}

#[derive(IntoElement)]
pub struct WindowControls {
    base: Div,
    leading: bool,
}

impl WindowControls {
    pub fn new(leading: bool) -> Self {
        Self {
            base: div(),
            leading,
        }
    }
}

impl Styled for WindowControls {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for WindowControls {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let supported = window.window_controls();
        let maximized = window.is_maximized();
        let theme = *cx.theme();
        let overrides = std::mem::take(self.base.style());

        let mut wanted = [
            supported.minimize.then_some(Control::Minimize),
            supported.maximize.then_some(match maximized {
                true => Control::Restore,
                false => Control::Maximize,
            }),
            Some(Control::Close),
        ];

        if self.leading {
            wanted.reverse();
        }

        let is_windows = cfg!(target_os = "windows");
        let caption = match is_windows {
            true => caption_font(cx),
            false => None,
        };
        let mut controls = self
            .base
            .flex()
            .flex_none()
            .items_center()
            .when_else(
                is_windows,
                |this| this.h_full().self_stretch().gap_0(),
                |this| this.gap_2(),
            )
            .when(!SYSTEM_ACTS, |this| {
                this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            })
            .children(wanted.into_iter().flatten().map(move |control| {
                let danger = control == Control::Close;

                div()
                    .id(control.id())
                    .role(gpui::accesskit::Role::Button)
                    .aria_label(control.name())
                    .group(control.id())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .when_else(
                        is_windows,
                        |this| this.h_full().aspect_square().rounded_none(),
                        |this| this.size(BUTTON).rounded(theme.radius).cursor_pointer(),
                    )
                    .occlude()
                    .window_control_area(control.area())
                    .hover(move |style| {
                        style.bg(match danger {
                            true => theme.danger,
                            false => theme.secondary_active,
                        })
                    })
                    .when(is_windows, |this| {
                        this.active(move |style| {
                            style.bg(match danger {
                                true => theme.danger_hover,
                                false => theme.secondary_hover,
                            })
                        })
                    })
                    .map(|this| match caption.clone() {
                        Some(caption) => this.child(
                            div()
                                .font(caption)
                                .text_size(CAPTION_GLYPH)
                                .line_height(CAPTION_GLYPH)
                                .relative()
                                .top(CAPTION_NUDGE)
                                .text_color(theme.muted_foreground)
                                .group_hover(control.id(), move |style| {
                                    style.text_color(match danger {
                                        true => theme.danger_foreground,
                                        false => theme.foreground,
                                    })
                                })
                                .child(control.caption_glyph()),
                        ),
                        None => this.child(
                            svg()
                                .path(icons::path(control.icon()))
                                .id("glyph")
                                .size(GLYPH)
                                .flex_none()
                                .text_color(theme.muted_foreground)
                                .group_hover(control.id(), move |style| {
                                    style.text_color(match danger {
                                        true => theme.danger_foreground,
                                        false => theme.foreground,
                                    })
                                }),
                        ),
                    })
                    .when(!control.system(), |this| {
                        this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                match control {
                                    Control::Minimize => window.minimize_window(),
                                    Control::Maximize | Control::Restore => window.zoom_window(),
                                    Control::Close => window.remove_window(),
                                }
                            })
                    })
                    // The system handles a press on its own maximize, which a screen reader cannot
                    // reach, so the click action zooms the window itself. Zooming never restores,
                    // so Restore gets no click action.
                    .when(control == Control::Maximize && control.system(), |this| {
                        this.on_a11y_action(gpui::accesskit::Action::Click, |_, window, _| {
                            window.zoom_window()
                        })
                    })
            }));
        controls.style().refine(&overrides);
        controls
    }
}

/// The first caption icon font installed on this system, or `None` when there is neither and the
/// controls draw their SVG icons. GPUI reads a font's fallbacks only once the font itself loads,
/// so each family is looked up by name. The answer is kept for the rest of the run.
fn caption_font(cx: &App) -> Option<Font> {
    static FAMILY: OnceLock<Option<&'static str>> = OnceLock::new();
    let family = FAMILY.get_or_init(|| {
        let installed = cx.text_system().all_font_names();
        let found = CAPTION_FONTS.into_iter().find(|family| {
            installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(family))
        });
        if found.is_none() {
            log::warn!("controls: no caption icon font is installed, drawing the icons instead");
        }
        found
    });
    family.map(font)
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn resize_handle(edge: ResizeEdge) -> Div {
    let handle = div()
        .absolute()
        .cursor(match edge {
            ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
            ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
            ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
            ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
        })
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            window.start_window_resize(edge);
        });

    match edge {
        ResizeEdge::Top => handle.top_0().left_0().right_0().h(RESIZE_EDGE),
        ResizeEdge::Bottom => handle.bottom_0().left_0().right_0().h(RESIZE_EDGE),
        ResizeEdge::Left => handle.top_0().bottom_0().left_0().w(RESIZE_EDGE),
        ResizeEdge::Right => handle.top_0().bottom_0().right_0().w(RESIZE_EDGE),
        ResizeEdge::TopLeft => handle.top_0().left_0().size(RESIZE_CORNER),
        ResizeEdge::TopRight => handle.top_0().right_0().size(RESIZE_CORNER),
        ResizeEdge::BottomLeft => handle.bottom_0().left_0().size(RESIZE_CORNER),
        ResizeEdge::BottomRight => handle.bottom_0().right_0().size(RESIZE_CORNER),
    }
}
