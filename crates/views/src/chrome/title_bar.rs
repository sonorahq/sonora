#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui::Decorations;
use gpui::prelude::*;
use gpui::{
    AnyView, Context, Entity, EventEmitter, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Render,
};
use gpui::{Window, div, px};
use ui::WindowControls;
use ui::{ActiveTheme as _, Button};

use crate::chrome::SidebarRight;
use router::Navigation;
use state::{AppSettings, Sonora};

const SYSTEM_ZOOMS: bool = cfg!(target_os = "windows");

#[cfg(target_os = "macos")]
const TITLE_BAR_LEFT_INSET: f32 = 74.;
#[cfg(not(target_os = "macos"))]
const TITLE_BAR_LEFT_INSET: f32 = 12.;

#[derive(Clone, PartialEq)]
pub(crate) struct TitleBarOptions {
    pub navigation: bool,
    pub sidebar_open: bool,
    pub sidebar_right: Option<bool>,
    pub offset: Pixels,
    pub border: bool,
    pub content: Option<AnyView>,
}

impl Default for TitleBarOptions {
    fn default() -> Self {
        Self {
            navigation: false,
            sidebar_open: false,
            sidebar_right: None,
            offset: Pixels::ZERO,
            border: true,
            content: None,
        }
    }
}

pub(crate) enum TitleBarEvent {
    ToggleSidebar,
    ToggleSidebarRight,
}

pub(crate) struct TitleBar {
    navigation: Entity<Navigation>,
    settings: Entity<AppSettings>,
    options: TitleBarOptions,
    grabbed: bool,
}

impl EventEmitter<TitleBarEvent> for TitleBar {}

impl TitleBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let navigation = router::trail(cx);
        let settings = Sonora::global(cx).settings.clone();

        cx.observe(&navigation, |_, _, cx| cx.notify()).detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        Self {
            navigation,
            settings,
            options: TitleBarOptions::default(),
            grabbed: false,
        }
    }

    pub fn set_options(&mut self, options: TitleBarOptions, cx: &mut Context<Self>) {
        if self.options == options {
            return;
        }
        self.options = options;
        cx.notify();
    }

    fn history(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hover = cx.theme().sidebar_accent;
        let navigation = self.navigation.read(cx);
        let (can_back, can_forward) = (navigation.can_go_back(), navigation.can_go_forward());
        let back = self.navigation.clone();
        let forward = self.navigation.clone();

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("history-back")
                    .ghost()
                    .icon("icons/chevron-left.svg")
                    .tooltip("nav-back")
                    .disabled(!can_back)
                    .size_8()
                    .px_0()
                    .when(can_back, |button| {
                        button
                            .hover(move |style| style.bg(hover))
                            .active(move |style| style.bg(hover))
                    })
                    .on_click(move |_, _, cx| {
                        back.update(cx, |navigation, cx| navigation.back(cx))
                    }),
            )
            .child(
                Button::new("history-forward")
                    .ghost()
                    .icon("icons/chevron-right.svg")
                    .tooltip("nav-forward")
                    .disabled(!can_forward)
                    .size_8()
                    .px_0()
                    .when(can_forward, |button| {
                        button
                            .hover(move |style| style.bg(hover))
                            .active(move |style| style.bg(hover))
                    })
                    .on_click(move |_, _, cx| {
                        forward.update(cx, |navigation, cx| navigation.forward(cx))
                    }),
            )
    }

    fn lyrics_toggle(&self, open: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_none()
            .items_center()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("sidebar-right-toggle")
                    .ghost()
                    .small()
                    .icon(match open {
                        true => "icons/panel-right-close.svg",
                        false => "icons/panel-right-open.svg",
                    })
                    .tooltip("nav-sidebar-right")
                    .selected(open)
                    .on_click(
                        cx.listener(|_, _, _, cx| cx.emit(TitleBarEvent::ToggleSidebarRight)),
                    ),
            )
    }

    fn toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let icon = match self.options.sidebar_open {
            true => "icons/panel-left-close.svg",
            false => "icons/panel-left-open.svg",
        };

        div()
            .flex()
            .flex_none()
            .items_center()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("sidebar-toggle")
                    .ghost()
                    .flex()
                    .small()
                    .icon(icon)
                    .tooltip("nav-sidebar")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(TitleBarEvent::ToggleSidebar))),
            )
    }
}

impl Render for TitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let height = ui::snapped(theme.metrics.title_bar, window);
        let navigation = self.options.navigation;
        let offset = match navigation {
            true => self.options.offset,
            false => Pixels::ZERO,
        };
        let content = self.options.content.clone();
        let settings = self.settings.read(cx);
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let controls = matches!(window.window_decorations(), Decorations::Client { .. });
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let controls = settings.window_controls();
        let decorated = cfg!(not(target_os = "macos")) && controls;
        let leading = decorated && settings.controls_on_left();

        div()
            .flex()
            .items_center()
            .w_full()
            .h(height)
            .flex_none()
            .when(!theme.transparent, |this| this.bg(theme.background))
            .when(self.options.border, |this| {
                this.border_b_1().border_color(theme.title_bar_border)
            })
            .window_control_area(gpui::WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(
                    |this, event: &MouseDownEvent, window, _| match event.click_count {
                        1 => this.grabbed = true,
                        2 if !SYSTEM_ZOOMS => window.zoom_window(),
                        _ => {}
                    },
                ),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.grabbed = false),
            )
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, _| this.grabbed = false))
            .on_mouse_move(cx.listener(|this, _: &MouseMoveEvent, window, _| {
                if this.grabbed {
                    this.grabbed = false;
                    window.start_window_move();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .when_else(
                        leading,
                        |this| this.pl_2(),
                        |this| this.pl(px(TITLE_BAR_LEFT_INSET)),
                    )
                    .pr_3()
                    .gap_1()
                    .when(offset > Pixels::ZERO, |this| this.w(offset))
                    .when(leading, |this| this.child(WindowControls::new(true)))
                    .when(navigation, |this| this.child(self.toggle(cx))),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .items_center()
                    .when(navigation, |this| this.child(self.history(cx)))
                    .children(content)
                    .pr_3(),
            )
            .when_some(
                self.options
                    .sidebar_right
                    .filter(|_| SidebarRight::available(window)),
                |this, open| {
                    this.child(div().flex_none().pr_3().child(self.lyrics_toggle(open, cx)))
                },
            )
            .when(decorated && !leading, |this| {
                this.child(
                    div()
                        .flex_none()
                        .when(cfg!(target_os = "windows"), |this| {
                            this.h_full().self_stretch()
                        })
                        .when(!cfg!(target_os = "windows"), |this| this.pr_2())
                        .child(WindowControls::new(false)),
                )
            })
    }
}
