#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui::Decorations;
use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Window, div};
use gpui::{
    AnyView, Context, Entity, EventEmitter, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Render,
};
use ui::TrafficLightControls;
use ui::WindowControls;
use ui::{ActiveTheme as _, Button, Spin};

use crate::chrome::SidebarRight;
use input::Refresh;
use router::Navigation;
use state::{AppSettings, Sonora};

const SYSTEM_ZOOMS: bool = cfg!(target_os = "windows");

/// Tells whether the page a refresh reloaded is still waiting for its data.
pub(crate) type Busy = Box<dyn Fn(&App) -> bool>;

/// The height of AppKit's traffic-light buttons, which the bar centres them by.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS_SIZE: f32 = 14.;
/// The extra room round corners leave before the traffic lights, since their wider curve
/// otherwise crowds the close button.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS_ROUND_PAD: f32 = 4.;
/// The bar's bottom border, which sits inside its height and so out of the centring.
#[cfg(target_os = "macos")]
const TITLE_BAR_BORDER: f32 = 1.;
/// Where hidden traffic lights go: past the window's left edge, where nothing draws or hits them.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS_HIDDEN: f32 = -200.;
/// The width of the three traffic lights and the gap the title bar leaves after them.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS_SPAN: f32 = 65.;

#[derive(Clone, PartialEq)]
pub(crate) struct TitleBarOptions {
    pub navigation: bool,
    pub sidebar_open: bool,
    pub sidebar_right: Option<bool>,
    pub offset: Pixels,
    pub border: bool,
    pub content: Option<AnyView>,
    /// Lets the fullscreen ambient background show through behind the controls.
    pub transparent: bool,
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
            transparent: false,
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
    /// Set by a refresh and dropped once the page has its data or the user moves on, so the
    /// refresh button turns for exactly that long.
    busy: Option<Busy>,
    /// The refresh icon's turn, which springs back to rest once `busy` is gone.
    spin: Spin,
}

impl EventEmitter<TitleBarEvent> for TitleBar {}

impl TitleBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let navigation = router::trail(cx);
        let settings = Sonora::global(cx).settings.clone();

        cx.observe(&navigation, |this, _, cx| {
            this.busy = None;
            cx.notify();
        })
        .detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        Self {
            navigation,
            settings,
            options: TitleBarOptions::default(),
            grabbed: false,
            busy: None,
            spin: Spin::default(),
        }
    }

    /// Turns the refresh button until `busy` says the reloaded page has its data.
    pub fn refreshing(&mut self, busy: Busy, cx: &mut Context<Self>) {
        self.busy = Some(busy);
        cx.notify();
    }

    pub fn set_options(&mut self, options: TitleBarOptions, cx: &mut Context<Self>) {
        if self.options == options {
            return;
        }
        self.options = options;
        cx.notify();
    }

    fn history(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.busy.as_ref().is_some_and(|busy| !busy(cx)) {
            self.busy = None;
        }
        match self.busy.is_some() {
            true => self.spin.start(),
            false => self.spin.stop(),
        }
        let turn = self.spin.turn(window, cx);
        let hover = cx.theme().sidebar_accent;
        let navigation = self.navigation.read(cx);
        let (can_back, can_forward) = (navigation.can_go_back(), navigation.can_go_forward());
        let refreshes = navigation.current().refreshes();
        let back = self.navigation.clone();
        let forward = self.navigation.clone();

        div()
            .id("titlebar-history")
            .role(Role::Toolbar)
            .aria_label(i18n::lookup("nav-history", None))
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
            .when(refreshes, |history| {
                history.child(
                    Button::new("history-refresh")
                        .ghost()
                        .icon("icons/refresh-cw.svg")
                        .turned(turn)
                        .tooltip("app-refresh")
                        .size_8()
                        .px_0()
                        .hover(move |style| style.bg(hover))
                        .active(move |style| style.bg(hover))
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(Refresh), cx)),
                )
            })
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
                    .aria_expanded(open)
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
                    .aria_expanded(self.options.sidebar_open)
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
        let window_fullscreen = window.is_fullscreen();
        let settings = self.settings.read(cx);
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let controls =
            matches!(window.window_decorations(), Decorations::Client { .. }) && !window_fullscreen;
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let controls = settings.window_controls() && !window_fullscreen;
        let inset = traffic_lights_inset(settings, height);
        let decorated = cfg!(not(target_os = "macos")) && controls;
        let leading = decorated && settings.controls_on_left();
        #[cfg(not(target_os = "macos"))]
        let traffic_light = settings.traffic_light_controls();
        #[cfg(target_os = "macos")]
        let traffic_light = false;
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(settings, cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<Pixels> = None;

        div()
            .id("title-bar")
            .role(Role::Banner)
            .flex()
            .items_center()
            .w_full()
            .h(height)
            .flex_none()
            .when_some(radius, |this, radius| this.rounded_t(radius))
            .when(!self.options.transparent && !theme.transparent, |this| {
                this.bg(theme.background)
            })
            // The height is a border box, so a bar that drops its border centres its controls
            // one pixel lower than one that keeps it. The edge is always an edge; only its
            // colour goes, and fullscreen's controls land where the workspace's do.
            .border_b_1()
            .border_color(match self.options.border {
                true => theme.title_bar_border,
                false => gpui::transparent_black(),
            })
            .window_control_area(gpui::WindowControlArea::Drag)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(
                    |this, event: &MouseDownEvent, window, _| match event.click_count {
                        1 => this.grabbed = true,
                        2 if !SYSTEM_ZOOMS => titlebar_double_click(window),
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
                    .map(|this| match (leading, inset) {
                        (true, _) => this.pl_2(),
                        (false, Some(inset)) => this.pl(inset),
                        (false, None) => this.pl_3(),
                    })
                    .pr_3()
                    .gap_1()
                    .when(offset > Pixels::ZERO, |this| this.w(offset))
                    .when(leading, |this| {
                        this.child(window_controls(true, traffic_light))
                    })
                    .when(navigation, |this| this.child(self.toggle(cx))),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .items_center()
                    .when(navigation, |this| this.child(self.history(window, cx)))
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
                        .when(cfg!(target_os = "windows") && !traffic_light, |this| {
                            this.h_full().self_stretch()
                        })
                        .when(!cfg!(target_os = "windows") || traffic_light, |this| {
                            this.pr_2()
                        })
                        .child(window_controls(false, traffic_light)),
                )
            })
    }
}

/// Where AppKit's traffic lights sit for the current settings and a title bar of `height`,
/// measured from the window's top-left corner. They are centred in the bar and kept as far from
/// the left edge as from the top, which centres them in the corner, with a little more room for
/// round corners. Hidden controls are moved off the window, since AppKit rebuilds them.
#[cfg(target_os = "macos")]
pub fn traffic_lights(settings: &AppSettings, height: Pixels) -> gpui::Point<Pixels> {
    let left = match settings.window_controls() {
        true => traffic_lights_left(settings, height),
        false => gpui::px(TRAFFIC_LIGHTS_HIDDEN),
    };
    gpui::point(left, traffic_lights_offset(height))
}

#[cfg(target_os = "macos")]
fn traffic_lights_left(settings: &AppSettings, height: Pixels) -> Pixels {
    let pad = match settings.window_rounding() {
        ui::Rounding::Round => gpui::px(TRAFFIC_LIGHTS_ROUND_PAD),
        _ => Pixels::ZERO,
    };
    traffic_lights_offset(height) + pad
}

#[cfg(target_os = "macos")]
fn traffic_lights_offset(height: Pixels) -> Pixels {
    let room = height - gpui::px(TITLE_BAR_BORDER + TRAFFIC_LIGHTS_SIZE);
    room / 2.
}

/// How far past the traffic lights the title bar's content starts while macOS shows them, or
/// `None` when it takes the same padding as the right edge.
fn traffic_lights_inset(settings: &AppSettings, height: Pixels) -> Option<Pixels> {
    #[cfg(target_os = "macos")]
    if settings.window_controls() {
        return Some(traffic_lights_left(settings, height) + gpui::px(TRAFFIC_LIGHTS_SPAN));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (settings, height);
    None
}

fn window_controls(leading: bool, traffic_light: bool) -> AnyElement {
    match traffic_light {
        true => TrafficLightControls::new(leading).into_any_element(),
        false => WindowControls::new(leading).into_any_element(),
    }
}

/// Runs the title bar's double-click action. macOS performs the action set in System Settings;
/// the window zooms everywhere else, since there is no such preference.
fn titlebar_double_click(window: &Window) {
    #[cfg(target_os = "macos")]
    window.titlebar_double_click();
    #[cfg(not(target_os = "macos"))]
    window.zoom_window();
}
