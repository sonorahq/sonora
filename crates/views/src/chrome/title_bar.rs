#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use gpui::Decorations;
use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{AnyElement, App, Point, SharedString, Window, div, px};
use gpui::{
    AnyView, Context, Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Render,
};
use ui::TrafficLightControls;
use ui::WindowControls;
use ui::{ActiveTheme as _, Badge, Button, MenuItem, Picker, Popovers, Popup, Spin};

use crate::chrome::SidebarRight;
use crate::chrome::downloads::DownloadsButton;
use crate::chrome::panels::{self, Customizing};
use i18n::t;
use input::{CustomizePanels, Refresh, ToggleSidebar};
use music::SignIn;
use router::{Destination, Navigation, Screen, SettingsPage, navigate, open_settings};
use state::{AppSettings, Sonora, Tool, Zone};

const SYSTEM_ZOOMS: bool = cfg!(target_os = "windows");
/// The rule between the current screen's controls and the account and sidebar segment.
const SEGMENT_RULE: Pixels = px(1.);

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
    pub border: bool,
    /// Gives the bar the chrome's colour, for a shell whose page lies on it as a sheet.
    pub sheet: bool,
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
            border: true,
            sheet: false,
            content: None,
            transparent: false,
        }
    }
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
    downloads: Entity<DownloadsButton>,
    /// The account menu's group, so it closes when another of the bar's menus opens.
    popovers: Popovers,
    /// Where a right click on the bar opened its menu.
    menu_at: Option<Point<Pixels>>,
}

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
        cx.observe(&Sonora::global(cx).session.clone(), |_, _, cx| cx.notify())
            .detach();
        let customizing = Customizing::entity(cx);
        cx.observe(&customizing, |_, _, cx| cx.notify()).detach();
        Self {
            navigation,
            settings,
            options: TitleBarOptions::default(),
            grabbed: false,
            busy: None,
            spin: Spin::default(),
            downloads: cx.new(DownloadsButton::new),
            popovers: Popovers::default(),
            menu_at: None,
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

    /// One of the history buttons, which lights up on hover like the sidebar's rows.
    fn step(&self, tool: Tool, cx: &App) -> AnyElement {
        let hover = cx.theme().sidebar_accent;
        let navigation = self.navigation.read(cx);
        let (id, icon, tip, can) = match tool {
            Tool::Back => (
                "history-back",
                "icons/chevron-left.svg",
                "nav-back",
                navigation.can_go_back(),
            ),
            _ => (
                "history-forward",
                "icons/chevron-right.svg",
                "nav-forward",
                navigation.can_go_forward(),
            ),
        };
        let trail = self.navigation.clone();

        Button::new(id)
            .ghost()
            .icon(icon)
            .tooltip(tip)
            .disabled(!can)
            .size_8()
            .px_0()
            .when(can, |button| {
                button
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(hover))
            })
            .on_click(move |_, _, cx| {
                trail.update(cx, |navigation, cx| match tool {
                    Tool::Back => navigation.back(cx),
                    _ => navigation.forward(cx),
                })
            })
            .into_any_element()
    }

    /// The refresh button while the page can reload, turning until the reload has its data.
    fn refresh(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.navigation.read(cx).current().refreshes() {
            return None;
        }
        let turn = self.spin.turn(window, cx);
        let hover = cx.theme().sidebar_accent;

        Some(
            Button::new("history-refresh")
                .ghost()
                .icon("icons/refresh-cw.svg")
                .turned(turn)
                .tooltip("app-refresh")
                .size_8()
                .px_0()
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(hover))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(Refresh), cx))
                .into_any_element(),
        )
    }

    /// The account button and the menu it opens: the connected services to switch between,
    /// guest mode when a provider offers it, a way to manage them, then History, Settings and
    /// the way into arranging the bars.
    fn account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = Sonora::global(cx).session.clone();
        let guest = session.read(cx).providers().find(|info| {
            info.options
                .iter()
                .any(|option| matches!(option, SignIn::Anonymous))
        });
        let mut services: Vec<MenuItem> = session
            .read(cx)
            .connected()
            .map(|info| {
                let slug = info.slug;
                let session = session.clone();

                MenuItem::new(SharedString::from(format!("account-{slug}")), info.name)
                    .icon(crate::shared::provider_logo(slug))
                    .selected(info.active)
                    .on_click(move |_, _, cx| {
                        let before = session.read(cx).provider_slug();
                        session.update(cx, |session, cx| session.switch(slug, cx));
                        if session.read(cx).provider_slug() != before {
                            open_startup(cx);
                        }
                    })
            })
            .collect();
        if let Some(info) = guest {
            let (slug, stored) = (info.slug, info.guest);
            services.push(
                MenuItem::new("account-guest", t!("login-guest-title"))
                    .icon("icons/hat-glasses.svg")
                    .selected(info.active && info.guest)
                    .on_click(move |_, _, cx| {
                        let before = session.read(cx).provider_slug();
                        let idle = !session.read(cx).is_pending();
                        session.update(cx, |session, cx| match stored {
                            true => session.switch(slug, cx),
                            false => session.sign_in(slug, SignIn::Anonymous, cx),
                        });
                        // an anonymous sign-in only takes over the provider once it lands, so a
                        // started one counts as the switch
                        let switched = match stored {
                            true => session.read(cx).provider_slug() != before,
                            false => idle && session.read(cx).is_pending() && before != Some(slug),
                        };
                        if switched {
                            open_startup(cx);
                        }
                    }),
            );
        }

        Picker::icon("account", &self.popovers, "icons/circle-user-round.svg")
            .tooltip("tool-account")
            .width(Picker::REGULAR)
            .items(services)
            .item(MenuItem::separator("account-services"))
            .item(
                MenuItem::new("account-manage", t!("account-manage"))
                    .icon("icons/user-round.svg")
                    .on_click(|_, _, cx| {
                        navigate(Destination::Settings(SettingsPage::Accounts), cx)
                    }),
            )
            .item(MenuItem::separator("account-pages"))
            .item(
                MenuItem::new("account-history", t!("nav-history"))
                    .icon("icons/rotate-ccw-clock.svg")
                    .on_click(|_, _, cx| navigate(Destination::History, cx)),
            )
            .item(
                MenuItem::new("account-settings", t!("nav-settings"))
                    .icon("icons/settings.svg")
                    .on_click(|_, _, cx| open_settings(cx)),
            )
            .item(
                MenuItem::new("account-customize", t!("panels-customize"))
                    .icon("icons/pencil.svg")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(CustomizePanels), cx)
                    }),
            )
    }

    /// The thin upright rule that sets the account and sidebar segment apart from the controls
    /// of the current screen.
    fn segment_rule(cx: &App) -> AnyElement {
        let theme = cx.theme();

        div()
            .flex_none()
            .w(SEGMENT_RULE)
            .h(theme.metrics.control_small / 2.)
            .mx_1()
            .bg(theme.border)
            .into_any_element()
    }

    fn toggle(&self) -> AnyElement {
        let icon = match self.options.sidebar_open {
            true => "icons/panel-left-close.svg",
            false => "icons/panel-left-open.svg",
        };

        Button::new("sidebar-toggle")
            .ghost()
            .flex()
            .small()
            .icon(icon)
            .tooltip("nav-sidebar")
            .aria_expanded(self.options.sidebar_open)
            .on_click(|_, window, cx| window.dispatch_action(Box::new(ToggleSidebar), cx))
            .into_any_element()
    }

    /// The bar's tools in the order the listener arranged them, with the current screen's own
    /// controls where the page tools stand.
    fn tools(
        &mut self,
        content: Option<AnyView>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let panels = self.settings.read(cx).panels();
        let mut content = content;
        let mut badged = !cfg!(debug_assertions);
        let mut tools: Vec<(Tool, AnyElement)> = Vec::new();
        for &tool in panels.zone(Zone::TitleBar) {
            let element = match tool {
                Tool::SidebarToggle => Some(self.toggle()),
                Tool::Back | Tool::Forward => Some(self.step(tool, cx)),
                Tool::Refresh => self.refresh(window, cx),
                Tool::Downloads => Some(self.downloads.clone().into_any_element()),
                // The screen's controls handle their own presses, and a press between them
                // still moves the window.
                Tool::PageTools => {
                    let content = content.take().map(|content| content.into_any_element());
                    if let Some(content) = content {
                        tools.push((tool, content));
                    }
                    continue;
                }
                Tool::Account => Some(self.account(cx).into_any_element()),
                Tool::Separator => Some(Self::segment_rule(cx)),
                Tool::Spacer => {
                    // The debug tag stands where the bar's leading run ends. A spacer ends a
                    // run anyway, so it never changes which separators show.
                    if !badged {
                        badged = true;
                        tools.push((Tool::Spacer, dev_badge().into_any_element()));
                    }
                    tools.push((tool, div().flex_1().into_any_element()));
                    continue;
                }
                tool => panels::icon_tool(tool, Zone::TitleBar, window, cx),
            };
            if let Some(element) = element {
                tools.push((tool, held(element)));
            }
        }
        if !badged {
            tools.push((Tool::Spacer, dev_badge().into_any_element()));
        }
        // The downloads button draws nothing while nothing downloads, so it never earns a
        // separator.
        panels::tidy(tools, |tool| tool == Tool::Downloads)
    }

    fn open_menu(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if window.default_prevented() || !self.options.navigation || Customizing::editing(cx) {
            return;
        }
        self.menu_at = Some(event.position);
        cx.notify();
    }
}

impl Render for TitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.busy.as_ref().is_some_and(|busy| !busy(cx)) {
            self.busy = None;
        }
        match self.busy.is_some() {
            true => self.spin.start(),
            false => self.spin.stop(),
        }
        let navigation = self.options.navigation;
        let editing = navigation && Customizing::editing(cx);
        if editing {
            Customizing::animate(window, cx);
        }
        let theme = cx.theme();
        let height = ui::snapped(theme.metrics.title_bar, window);
        let sidebar_right = self
            .options
            .sidebar_right
            .filter(|_| SidebarRight::available(window));
        let content = self.options.content.clone();
        let window_fullscreen = window.is_fullscreen();
        let settings = self.settings.read(cx);
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let controls =
            matches!(window.window_decorations(), Decorations::Client { .. }) && !window_fullscreen;
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let controls = settings.window_controls() && !window_fullscreen;
        // Fullscreen hides the traffic lights, so the first tool sits as far from the left edge
        // as the last one does from the right.
        let inset = traffic_lights_inset(settings, height).filter(|_| !window_fullscreen);
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
        let background = match self.options.sheet {
            true => theme.sidebar,
            false => theme.background,
        };
        let transparent = self.options.transparent || theme.transparent;
        let border = match self.options.border {
            true => theme.title_bar_border,
            false => gpui::transparent_black(),
        };

        let row = div()
            .id("title-bar-tools")
            .flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .items_center()
            // The outline of a bar being arranged keeps clear of the window's edges, so its tools
            // keep their places and their badges stay inside the window.
            .map(|this| match (editing, leading, inset) {
                (false, true, _) => this.pl_1(),
                (false, false, Some(inset)) => this.pl(inset),
                (false, false, None) => this.pl_3(),
                (true, true, _) => this.ml_1(),
                (true, false, Some(inset)) => this.ml(inset),
                (true, false, None) => this.ml_2(),
            })
            .map(|this| match editing {
                true => this.mr_2().px_1(),
                false => this.pr_3(),
            });
        let row = match (navigation, editing) {
            (true, true) => panels::zone(Zone::TitleBar, row, cx)
                .py_0p5()
                .children(panels::tiles(Zone::TitleBar, false, window, cx)),
            (true, false) => {
                let tools = self.tools(content, window, cx);
                row.children(tools)
            }
            (false, _) => row
                .child(div().flex_1())
                .children(content.map(|content| content.into_any_element()))
                .children(
                    sidebar_right
                        .and_then(|_| panels::icon_tool(Tool::Panel, Zone::TitleBar, window, cx))
                        .map(held),
                ),
        };
        let menu = self.menu_at.map(|at| {
            Popup::new(at, panels::menu()).on_close(cx.listener(|this, _, _, cx| {
                this.menu_at = None;
                cx.notify();
            }))
        });

        div()
            .id("title-bar")
            .role(Role::Banner)
            .flex()
            .items_center()
            .w_full()
            .h(height)
            .flex_none()
            .when_some(radius, |this, radius| this.rounded_t(radius))
            .when(!transparent, |this| this.bg(background))
            // The height is a border box, so a bar that drops its border centres its controls
            // one pixel lower than one that keeps it. The edge is always an edge; only its
            // colour goes, and fullscreen's controls land where the workspace's do.
            .border_b_1()
            .border_color(border)
            // While the bars are arranged a press on a tile drags the tile, so the platform must
            // not take the bar for a caption it moves the window by.
            .when(!editing, |this| {
                this.window_control_area(gpui::WindowControlArea::Drag)
            })
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
            .on_mouse_down(MouseButton::Right, cx.listener(Self::open_menu))
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
            .when(leading, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .pl_2()
                        .child(window_controls(true, traffic_light)),
                )
            })
            .child(row)
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
            .children(menu)
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

/// Keeps a press on one of the bar's controls from moving the window.
fn held(element: AnyElement) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(element)
        .into_any_element()
}

/// The tag a debug build carries in its title bar, so it is never mistaken for a release.
fn dev_badge() -> impl IntoElement {
    div()
        .id("dev-badge")
        .role(Role::Label)
        .aria_label(t!("app-dev-build"))
        .flex_none()
        .child(Badge::text(t!("app-dev-badge")))
}

/// Opens the screen chosen as the startup page in Settings, where a newly switched provider lands.
fn open_startup(cx: &mut App) {
    let startup = Sonora::global(cx).settings.read(cx).startup().to_owned();
    let screen = Screen::from_id(&startup).unwrap_or(Screen::Home);
    navigate(screen.destination(), cx);
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
