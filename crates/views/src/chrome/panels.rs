//! Arranging the title bar, the sidebar and the player bar.
//!
//! The three bars draw the [`Tool`]s the settings give each of them. While [`Customizing`] is
//! on, every bar draws its tools as jiggling tiles that can be dragged within the bar, into
//! another bar or onto the palette, which takes them out. The palette floats over the page and
//! offers what no bar holds.

use std::f32::consts::TAU;
use std::time::Instant;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, DragMoveEvent, Entity, FontWeight, Global, Hsla, MouseButton,
    Pixels, Point, Render, SharedString, Stateful, Window, div, px, svg,
};
use i18n::t;
use input::{
    CustomizePanels, NavigateBack, NavigateForward, ToggleLyrics, TogglePanel, ToggleQueue,
};
use router::{Destination, LibraryTab, SettingsPage, navigate, open_settings};
use state::{SideTab, Sonora, Tool, Zone};
use ui::{ActiveTheme as _, Button, Menu, MenuItem, Text};

use crate::chrome::SidebarRight;
use crate::shared::nav::{nav_icon, nav_row};

/// One jiggle, there and back, in seconds, and how far a tile tilts either way, in degrees.
const JIGGLE_PERIOD: f32 = 0.3;
const JIGGLE_DEGREES: f32 = 2.;
/// How much the period of one tile differs from its neighbours', so they never sway in step.
const JIGGLE_SPREAD: f32 = 0.09;
/// The golden angle, which spreads the tiles' phases evenly however many there are.
const PHASE_STEP: f32 = 2.399;
/// The round remove and anchor badges a tile shows under the pointer, their glyph, and how far
/// past a tile's top right corner they sit.
const BADGE: Pixels = px(16.);
const BADGE_ICON: Pixels = px(10.);
const BADGE_OFFSET: Pixels = px(-4.);
/// How far past its edges a tile still aims a dragged tool beside it, half the bars' `gap_1`.
const SLACK: Pixels = px(2.);
/// The width of the marker where a dragged tool would land, and the rule a separator draws.
const MARKER: Pixels = px(2.);
const RULE: Pixels = px(1.);
/// How faded a tile is while it is picked up, or while its tool has nothing to show right now,
/// and a bar that cannot take the tool in flight.
const LIFTED: f32 = 0.3;
const RESTING: f32 = 0.55;
const BARRED: f32 = 0.3;
/// The palette's width as a share of the cover metric, how many tiles a row of it holds, few
/// enough for a name like Flexible space to fit on one line, and a tile's height as a share of a
/// control's.
const PALETTE_WIDTH: f32 = 3.2;
const COLUMNS: u16 = 3;
const TILE_HEIGHT: f32 = 2.2;
/// The hover group of a tile, which shows its badge.
const TILE_GROUP: &str = "panel-tile";
/// How transparent the outline of a bar being arranged is.
const OUTLINE: f32 = 0.45;
/// One pulse, out and back, in seconds, of a bar that would take the tool in flight, how strongly
/// such a bar is tinted at the height of it, and how strongly the bar the tool would land in is.
const PULSE_PERIOD: f32 = 1.1;
const GLOW: f32 = 0.14;
const AIMED_GLOW: f32 = 0.24;

/// Whether the bars are being arranged, and the drag in flight while they are.
pub(crate) struct Customizing {
    since: Option<Instant>,
    /// The bar and the gap a dragged tool would land in.
    aim: Option<(Zone, usize)>,
    /// The tool being dragged and where it was picked up from, `None` for one from the palette.
    held: Option<(Tool, Option<(Zone, usize)>)>,
    /// When a tool was last picked up, which the bars that would take it pulse from.
    picked: Option<Instant>,
}

struct Installed(Entity<Customizing>);

impl Global for Installed {}

/// Where a tile stands: its bar, its place in the bar, which copy of a repeating tool it is
/// and how many tools the bar holds.
#[derive(Clone, Copy)]
struct Seat {
    zone: Zone,
    index: usize,
    copy: usize,
    count: usize,
}

/// A tool on its way to a bar or back to the palette.
#[derive(Clone)]
pub(crate) struct DraggedTool {
    tool: Tool,
    from: Option<(Zone, usize)>,
    offset: Point<Pixels>,
}

impl Customizing {
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let customizing = cx.new(|_| Self {
                since: None,
                aim: None,
                held: None,
                picked: None,
            });
            cx.set_global(Installed(customizing));
        }
        cx.global::<Installed>().0.clone()
    }

    fn read(cx: &App) -> Option<&Self> {
        cx.try_global::<Installed>()
            .map(|installed| installed.0.read(cx))
    }

    pub fn editing(cx: &App) -> bool {
        Self::read(cx).is_some_and(|this| this.since.is_some())
    }

    pub fn start(cx: &mut App) {
        Self::entity(cx).update(cx, |this, cx| {
            if this.since.is_none() {
                this.since = Some(Instant::now());
                cx.notify();
            }
        });
    }

    pub fn stop(cx: &mut App) {
        if !Self::editing(cx) {
            return;
        }
        Self::entity(cx).update(cx, |this, cx| {
            this.since = None;
            this.aim = None;
            this.held = None;
            cx.notify();
        });
    }

    /// Where a dragged tool would land right now.
    fn aim(cx: &App) -> Option<(Zone, usize)> {
        Self::read(cx)
            .and_then(|this| this.aim)
            .filter(|_| cx.has_active_drag())
    }

    fn held(cx: &App) -> Option<(Tool, Option<(Zone, usize)>)> {
        Self::read(cx)
            .and_then(|this| this.held)
            .filter(|_| cx.has_active_drag())
    }

    fn set_aim(aim: Option<(Zone, usize)>, cx: &mut App) {
        Self::entity(cx).update(cx, |this, cx| {
            if this.aim != aim {
                this.aim = aim;
                cx.notify();
            }
        });
    }

    fn hold(tool: Tool, from: Option<(Zone, usize)>, cx: &mut App) {
        Self::entity(cx).update(cx, |this, cx| {
            this.held = Some((tool, from));
            this.picked = Some(Instant::now());
            this.aim = None;
            cx.notify();
        });
    }

    /// Keeps a bar that is being arranged drawing: every frame while the tiles jiggle, and while
    /// a drag is in flight, so the bar lets go of its drop marker the frame after the drag ends
    /// wherever it lands. Called once from the render of each bar.
    pub fn animate(window: &Window, cx: &App) {
        if Self::editing(cx) && (!cx.reduce_motion() || cx.has_active_drag()) {
            window.request_animation_frame();
        }
    }

    /// How far the tile `seed` is tilted now, as a part of a full turn.
    fn jiggle(seed: usize, cx: &App) -> f32 {
        let Some(since) = Self::read(cx).and_then(|this| this.since) else {
            return 0.;
        };
        if cx.reduce_motion() {
            return 0.;
        }
        let period = JIGGLE_PERIOD * (1. + (seed % 3) as f32 * JIGGLE_SPREAD);
        let phase = seed as f32 * PHASE_STEP;
        (since.elapsed().as_secs_f32() * TAU / period + phase).sin() * JIGGLE_DEGREES / 360.
    }

    /// How lit a bar that would take the tool in flight is now, from 0 to 1. It starts lit the
    /// moment the tool is picked up, and holds lit when the motion setting stills the pulse.
    fn pulse(cx: &App) -> f32 {
        let Some(picked) = Self::read(cx).and_then(|this| this.picked) else {
            return 1.;
        };
        if cx.reduce_motion() {
            return 1.;
        }
        0.5 + 0.5 * (picked.elapsed().as_secs_f32() * TAU / PULSE_PERIOD).cos()
    }

    /// Puts the dragged tool where it was aimed in `zone`. gpui takes the drag out of `cx` before
    /// it calls a drop listener, so this reads the aim as it stands rather than through `aim`.
    fn land(zone: Zone, dragged: &DraggedTool, cx: &mut App) {
        let aim = Self::read(cx)
            .and_then(|this| this.aim)
            .filter(|(aimed, _)| *aimed == zone);
        Self::entity(cx).update(cx, |this, cx| {
            this.aim = None;
            this.held = None;
            cx.notify();
        });
        let Some((_, gap)) = aim else {
            return;
        };
        let settings = Sonora::global(cx).settings.clone();
        settings.update(cx, |settings, cx| {
            let mut panels = settings.panels();
            if panels.place(dragged.tool, dragged.from, zone, gap) {
                settings.set_panels(panels, cx);
            }
        });
    }

    fn remove(zone: Zone, index: usize, cx: &mut App) {
        let settings = Sonora::global(cx).settings.clone();
        settings.update(cx, |settings, cx| {
            let mut panels = settings.panels();
            if panels.remove(zone, index) {
                settings.set_panels(panels, cx);
            }
        });
    }

    fn add(tool: Tool, cx: &mut App) {
        let settings = Sonora::global(cx).settings.clone();
        settings.update(cx, |settings, cx| {
            let mut panels = settings.panels();
            if panels.add(tool).is_some() {
                settings.set_panels(panels, cx);
            }
        });
    }

    fn restore(cx: &mut App) {
        let settings = Sonora::global(cx).settings.clone();
        settings.update(cx, |settings, cx| {
            settings.set_panels(state::Panels::default(), cx)
        });
    }
}

impl DraggedTool {
    fn at(mut self, offset: Point<Pixels>) -> Self {
        self.offset = offset;
        self
    }
}

impl Render for DraggedTool {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let row = Customizing::aim(cx).is_some_and(|(zone, _)| zone == Zone::Sidebar);
        let chip = div()
            .flex()
            .items_center()
            .gap_2()
            .h(theme.metrics.control_small)
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.foreground)
            .shadow_lg()
            .child(glyph(self.tool, theme.text(Text::Body), cx))
            .map(|chip| match row {
                true => chip
                    .px_3()
                    .text_size(theme.text(Text::Label))
                    .child(i18n::lookup(label(self.tool), None)),
                false => chip.justify_center().w(theme.metrics.control_small),
            });

        div().pl(self.offset.x).pt(self.offset.y).child(chip)
    }
}

/// The Fluent key naming `tool`.
pub(crate) fn label(tool: Tool) -> &'static str {
    match tool {
        Tool::Home => "nav-home",
        Tool::Search => "nav-search",
        Tool::History => "nav-history",
        Tool::Settings => "nav-settings",
        Tool::Songs => "nav-songs",
        Tool::Albums => "nav-albums",
        Tool::Artists => "nav-artists",
        Tool::Playlists => "nav-playlists",
        Tool::Library => "sidebar-library",
        Tool::SidebarToggle => "nav-sidebar",
        Tool::Back => "nav-back",
        Tool::Forward => "nav-forward",
        Tool::Refresh => "app-refresh",
        Tool::Downloads => "panels-downloads",
        Tool::PageTools => "panels-page-tools",
        Tool::Account => "tool-account",
        Tool::Panel => "panels-side-panel",
        Tool::Lyrics => "lyrics-title",
        Tool::Queue => "queue-title",
        Tool::Sessions => "player-sessions",
        Tool::Spacer => "panels-spacer",
        Tool::Separator => "panels-separator",
    }
}

/// The Fluent key of the tooltip on the button of `tool`, which says what a press does where the
/// name alone would not.
fn tip(tool: Tool) -> &'static str {
    match tool {
        Tool::Panel => "nav-sidebar-right",
        tool => label(tool),
    }
}

/// The icon standing for `tool`, in a bar and on the palette.
pub(crate) fn icon(tool: Tool) -> &'static str {
    match tool {
        Tool::Home => "icons/house.svg",
        Tool::Search => "icons/search.svg",
        Tool::History => "icons/rotate-ccw-clock.svg",
        Tool::Settings => "icons/settings.svg",
        Tool::Songs => LibraryTab::Songs.icon(),
        Tool::Albums => LibraryTab::Albums.icon(),
        Tool::Artists => LibraryTab::Artists.icon(),
        Tool::Playlists => LibraryTab::Playlists.icon(),
        Tool::Library => "icons/library-big.svg",
        Tool::SidebarToggle => "icons/panel-left-open.svg",
        Tool::Back => "icons/chevron-left.svg",
        Tool::Forward => "icons/chevron-right.svg",
        Tool::Refresh => "icons/refresh-cw.svg",
        Tool::Downloads => "icons/download.svg",
        Tool::PageTools => "icons/sliders-horizontal.svg",
        Tool::Account => "icons/circle-user-round.svg",
        Tool::Panel => "icons/panel-right-open.svg",
        Tool::Lyrics => "icons/mic-vocal.svg",
        Tool::Queue => "icons/list-music.svg",
        Tool::Sessions => "icons/monitor-speaker.svg",
        Tool::Spacer => "icons/move-horizontal.svg",
        Tool::Separator => "icons/minus.svg",
    }
}

/// Where a navigation tool goes. Settings opens on the settings page last visited rather than
/// on the one named here.
pub(crate) fn route(tool: Tool) -> Option<Destination> {
    Some(match tool {
        Tool::Home => Destination::Home,
        Tool::Search => Destination::Search,
        Tool::History => Destination::History,
        Tool::Settings => Destination::Settings(SettingsPage::Providers),
        Tool::Songs => Destination::Library(LibraryTab::Songs),
        Tool::Albums => Destination::Library(LibraryTab::Albums),
        Tool::Artists => Destination::Library(LibraryTab::Artists),
        Tool::Playlists => Destination::Library(LibraryTab::Playlists),
        _ => return None,
    })
}

/// Opens the page of a navigation tool.
pub(crate) fn go(destination: &Destination, cx: &mut App) {
    match destination.in_settings() {
        true => open_settings(cx),
        false => navigate(destination.clone(), cx),
    }
}

/// Whether `tool` has anything to show at this width and with these settings. The right panel's
/// buttons need room for the panel, the lyrics button needs lyrics turned on, and Sessions needs
/// something to connect to.
pub(crate) fn shown(tool: Tool, window: &Window, cx: &App) -> bool {
    let settings = Sonora::global(cx).settings.read(cx);
    match tool {
        Tool::Lyrics => SidebarRight::available(window) && settings.lyrics_enabled(),
        Tool::Queue | Tool::Panel => SidebarRight::available(window),
        Tool::Sessions => {
            Sonora::global(cx).devices.read(cx).available()
                || Sonora::global(cx).listening.read(cx).available(cx)
        }
        _ => true,
    }
}

/// Drops the separators that would stand at either end of a run of tools. A spacer ends a run,
/// and a tool `hollow` says draws nothing right now does not count as a neighbour.
pub(crate) fn tidy(
    tools: Vec<(Tool, AnyElement)>,
    hollow: impl Fn(Tool) -> bool,
) -> Vec<AnyElement> {
    let solid = |tool: Tool| !matches!(tool, Tool::Spacer | Tool::Separator) && !hollow(tool);
    let ends = |tool: Tool| matches!(tool, Tool::Spacer | Tool::Separator);
    let kinds: Vec<Tool> = tools.iter().map(|(tool, _)| *tool).collect();
    let flanked = |index: usize| {
        let before = kinds[..index]
            .iter()
            .rev()
            .take_while(|tool| !ends(**tool))
            .any(|tool| solid(*tool));
        let after = kinds[index + 1..]
            .iter()
            .take_while(|tool| !ends(**tool))
            .any(|tool| solid(*tool));
        before && after
    };
    tools
        .into_iter()
        .enumerate()
        .filter(|(index, (tool, _))| *tool != Tool::Separator || flanked(*index))
        .map(|(_, (_, element))| element)
        .collect()
}

/// A tool that works in the title bar and the player bar alike, as a small icon button. `None`
/// for a tool the bar has to draw itself, or one with nothing to show right now.
pub(crate) fn icon_tool(tool: Tool, zone: Zone, window: &Window, cx: &App) -> Option<AnyElement> {
    if !shown(tool, window, cx) {
        return None;
    }
    let theme = *cx.theme();
    let id = SharedString::from(format!("{}-{}", zone.id(), tool.id()));
    let button = Button::new(id).ghost().small().icon(icon(tool));
    let button = match zone {
        Zone::PlayerBar => button.tooltip_above(tip(tool)),
        _ => button.tooltip(tip(tool)),
    };
    let tint = |on: bool| match on {
        true => theme.foreground,
        false => theme.muted_foreground,
    };
    let settings = Sonora::global(cx).settings.read(cx);
    let button = match tool {
        Tool::Back | Tool::Forward => {
            let navigation = router::trail(cx).read(cx);
            let (can, action): (bool, Box<dyn gpui::Action>) = match tool {
                Tool::Back => (navigation.can_go_back(), Box::new(NavigateBack)),
                _ => (navigation.can_go_forward(), Box::new(NavigateForward)),
            };
            button
                .disabled(!can)
                .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        }
        Tool::Panel => {
            let open = settings.sidebar_right_open();
            button
                .icon(match open {
                    true => "icons/panel-right-close.svg",
                    false => "icons/panel-right-open.svg",
                })
                .aria_expanded(open)
                .on_click(|_, window, cx| window.dispatch_action(Box::new(TogglePanel), cx))
        }
        Tool::Lyrics | Tool::Queue => {
            let side = match tool {
                Tool::Lyrics => SideTab::Lyrics,
                _ => SideTab::Queue,
            };
            let showing = settings.sidebar_right_open() && settings.sidebar_right_tab() == side;
            button
                .selected(showing)
                .tint(tint(showing))
                .on_click(move |_, window, cx| {
                    let action: Box<dyn gpui::Action> = match side {
                        SideTab::Lyrics => Box::new(ToggleLyrics),
                        _ => Box::new(ToggleQueue),
                    };
                    window.dispatch_action(action, cx);
                })
        }
        tool => {
            let destination = route(tool)?;
            let active = destination.same_section(&router::trail(cx).read(cx).current());
            button
                .tint(tint(active))
                .when(active, |button| button.aria_selected(true))
                .on_click(move |_, _, cx| go(&destination, cx))
        }
    };
    Some(button.into_any_element())
}

/// The menu a right click on a bar opens, with the way into arranging the bars.
pub(crate) fn menu() -> Menu {
    Menu::new("panels-menu").item(
        MenuItem::new("panels-customize", t!("panels-customize"))
            .icon("icons/pencil.svg")
            .on_click(|_, window, cx| window.dispatch_action(Box::new(CustomizePanels), cx)),
    )
}

/// Wraps the tiles of `zone` while the bars are arranged: a dashed outline that takes a dragged
/// tool, named for screen readers as the list it is. While a tool is in flight, every bar that
/// would take it pulses, the one it would land in holds lit and the rest fade.
pub(crate) fn zone(zone: Zone, base: Stateful<Div>, cx: &App) -> Stateful<Div> {
    let theme = *cx.theme();
    let held = Customizing::held(cx);
    let fits = held.is_none_or(|(tool, _)| tool.fits(zone));
    let aimed = Customizing::aim(cx).is_some_and(|(aimed, _)| aimed == zone);
    // The outline's and the tint's strength.
    let lit = match (held.is_some() && fits, aimed) {
        (false, _) => None,
        (true, true) => Some((1., AIMED_GLOW)),
        (true, false) => {
            let pulse = Customizing::pulse(cx);
            Some((OUTLINE + (1. - OUTLINE) * pulse, GLOW * pulse))
        }
    };
    let count = Sonora::global(cx)
        .settings
        .read(cx)
        .panels()
        .zone(zone)
        .len();

    base.role(Role::List)
        .aria_label(i18n::lookup(zone.key(), None))
        .relative()
        .rounded(theme.radius)
        .border_1()
        .border_dashed()
        .border_color(theme.primary.opacity(OUTLINE))
        .when_some(lit, |this, (outline, glow)| {
            this.border_color(theme.primary.opacity(outline))
                .bg(theme.primary.opacity(glow))
        })
        .when(!fits, |this| this.opacity(BARRED))
        .on_drag_move(move |event: &DragMoveEvent<DraggedTool>, _, cx| {
            let inside = event.bounds.contains(&event.event.position);
            let aimed = Customizing::aim(cx).map(|(aimed, _)| aimed);
            match (inside, aimed == Some(zone)) {
                (true, false) if event.drag(cx).tool.fits(zone) => {
                    Customizing::set_aim(Some((zone, count)), cx)
                }
                (false, true) => Customizing::set_aim(None, cx),
                _ => {}
            }
        })
        .can_drop(move |dragged, _, _| {
            dragged
                .downcast_ref::<DraggedTool>()
                .is_some_and(|dragged| dragged.tool.fits(zone))
        })
        .on_drop(move |dragged: &DraggedTool, _, cx| Customizing::land(zone, dragged, cx))
}

/// The tiles of `zone` while the bars are arranged, with the gap a dragged tool would land in.
/// `folded` draws the sidebar's tiles as the rail's icons.
pub(crate) fn tiles(zone: Zone, folded: bool, window: &Window, cx: &App) -> Vec<AnyElement> {
    let tools = Sonora::global(cx)
        .settings
        .read(cx)
        .panels()
        .zone(zone)
        .to_vec();
    let aim = Customizing::aim(cx)
        .filter(|(aimed, _)| *aimed == zone)
        .map(|(_, gap)| gap);
    let count = tools.len();
    let mut seen: Vec<Tool> = Vec::new();
    let mut tiles = Vec::new();
    for (index, tool) in tools.into_iter().enumerate() {
        if aim == Some(index) {
            tiles.push(marker(zone, cx));
        }
        let copy = seen.iter().filter(|held| **held == tool).count();
        seen.push(tool);
        let seat = Seat {
            zone,
            index,
            copy,
            count,
        };
        tiles.push(tile(tool, seat, folded, window, cx));
    }
    if aim.is_some_and(|gap| gap >= count) {
        tiles.push(marker(zone, cx));
    }
    tiles
}

/// One tool drawn as a tile: still unless the motion setting allows the jiggle, draggable, and
/// with the badge that takes it out of the bar.
fn tile(tool: Tool, seat: Seat, folded: bool, window: &Window, cx: &App) -> AnyElement {
    let Seat {
        zone,
        index,
        copy,
        count,
    } = seat;
    let theme = *cx.theme();
    let key = format!("{}-{}-{copy}", zone.id(), tool.id());
    let seed = Tool::ALL
        .iter()
        .position(|known| *known == tool)
        .unwrap_or_default()
        * 7
        + copy * 3
        + index;
    let turn = Customizing::jiggle(seed, cx);
    let lifted = Customizing::held(cx).is_some_and(|(_, from)| from == Some((zone, index)));
    let resting = !shown(tool, window, cx);
    let name = i18n::lookup(label(tool), None);
    let dragged = DraggedTool {
        tool,
        from: Some((zone, index)),
        offset: Point::default(),
    };

    let body = div()
        .id(SharedString::from(format!("panel-body-{key}")))
        .flex()
        .items_center()
        .when(zone == Zone::Sidebar, |this| this.w_full())
        .when(tool == Tool::Spacer, |this| this.flex_1())
        .cursor_grab()
        .child(face(tool, zone, &key, turn, folded, cx))
        .on_drag(dragged, move |dragged, offset, _, cx| {
            Customizing::hold(dragged.tool, dragged.from, cx);
            cx.new(|_| dragged.clone().at(offset))
        })
        .on_drag_move(move |event: &DragMoveEvent<DraggedTool>, _, cx| {
            let (bounds, position) = (event.bounds, event.event.position);
            // A tile answers for half the gap on either side too, so a drop between two tiles
            // lands between them rather than at the end of the bar.
            if !bounds.dilate(SLACK).contains(&position) || !event.drag(cx).tool.fits(zone) {
                return;
            }
            let after = match zone {
                Zone::Sidebar => position.y > bounds.center().y,
                _ => position.x > bounds.center().x,
            };
            Customizing::set_aim(Some((zone, index + usize::from(after))), cx);
        });

    // The badge waits for the pointer, so the bars rest clean while they jiggle. A tool that
    // cannot leave shows a lock there instead of a way out.
    let badge = match tool.anchored() {
        true => badge("icons/lock.svg", theme.secondary, theme.muted_foreground)
            .border_1()
            .border_color(theme.border),
        false => badge("icons/x.svg", theme.foreground, theme.background),
    };
    let badge = match tool.anchored() {
        true => badge.into_any_element(),
        false => badge
            .id(SharedString::from(format!("panel-remove-{key}")))
            .role(Role::Button)
            .aria_label(t!("panels-remove", name = name.as_ref()))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| Customizing::remove(zone, index, cx))
            .into_any_element(),
    };
    // A row of the full sidebar takes it at its end, where closing a row is expected, and an
    // icon at its top right corner, where a tab or a notice closes.
    let row = zone == Zone::Sidebar && !folded;
    let corner = div()
        .absolute()
        .invisible()
        .when(!cx.has_active_drag(), |this| {
            this.group_hover(TILE_GROUP, |style| style.visible())
        })
        .map(|this| match row {
            true => this.top_0().bottom_0().right_2().flex().items_center(),
            false => this.top(BADGE_OFFSET).right(BADGE_OFFSET),
        })
        .child(badge);

    div()
        .id(SharedString::from(format!("panel-tile-{key}")))
        .group(TILE_GROUP)
        .role(Role::ListItem)
        .aria_label(name)
        .when(tool.anchored(), |this| {
            this.aria_description(t!("panels-anchored"))
        })
        .aria_position_in_set(index + 1)
        .aria_size_of_set(count)
        .relative()
        .flex()
        .items_center()
        .map(|this| match (zone, tool) {
            (Zone::Sidebar, _) => this.w_full(),
            (_, Tool::Spacer) => this.flex_1().min_w(theme.metrics.control_small * 2.),
            _ => this.flex_none(),
        })
        // The bar behind a tile moves the window or opens its own menus on a press. The tile
        // never occludes, since that would hide the bar it stands in from a drop over it.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(lifted, |this| this.opacity(LIFTED))
        .when(resting && !lifted, |this| this.opacity(RESTING))
        .child(body)
        .child(corner)
        .into_any_element()
}

/// What a tile shows of its tool: the bar's own look for it, with the icon tilted by `turn`.
fn face(tool: Tool, zone: Zone, key: &str, turn: f32, folded: bool, cx: &App) -> AnyElement {
    let theme = *cx.theme();
    let id = SharedString::from(format!("panel-face-{key}"));
    let accent = theme.sidebar_accent;
    match (zone, tool) {
        (_, Tool::Separator) => rule(zone, cx).into_any_element(),
        (Zone::Sidebar, Tool::Library) if !folded => div()
            .flex()
            .items_center()
            .w_full()
            .h(theme.metrics.control_small)
            .px_3()
            .child(ui::eyebrow(i18n::lookup(label(tool), None), cx).font_weight(FontWeight::MEDIUM))
            .into_any_element(),
        (Zone::Sidebar, Tool::Library) => rule(zone, cx).into_any_element(),
        (Zone::Sidebar, _) if folded => {
            nav_icon(id, label(tool), icon(tool), theme.muted_foreground, accent)
                .turned(turn)
                .into_any_element()
        }
        (Zone::Sidebar, _) => nav_row(id, label(tool), theme.muted_foreground, accent)
            .icon(icon(tool))
            .turned(turn)
            .w_full()
            .into_any_element(),
        (_, Tool::Spacer) => div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(theme.metrics.control_small)
            .rounded(theme.radius)
            .border_1()
            .border_dashed()
            .border_color(theme.muted_foreground.opacity(OUTLINE))
            .text_size(theme.text(Text::Small))
            .text_color(theme.muted_foreground)
            .child(glyph(tool, theme.text(Text::Small), cx))
            .into_any_element(),
        (_, Tool::PageTools) => Button::new(id)
            .ghost()
            .small()
            .icon(icon(tool))
            .label(i18n::lookup(label(tool), None))
            .turned(turn)
            .tint(theme.muted_foreground)
            .into_any_element(),
        (zone, tool) => Button::new(id)
            .ghost()
            .small()
            .icon(icon(tool))
            .turned(turn)
            .map(|button| match zone {
                Zone::PlayerBar => button.tooltip_above(tip(tool)),
                _ => button.tooltip(tip(tool)),
            })
            .into_any_element(),
    }
}

/// The upright rule a separator draws in a row, or the level one it draws down the sidebar.
fn rule(zone: Zone, cx: &App) -> Div {
    let theme = *cx.theme();
    match zone {
        Zone::Sidebar => div()
            .flex()
            .items_center()
            .w_full()
            .h(theme.metrics.control_small / 2.)
            .px_2()
            .child(div().w_full().h(RULE).bg(theme.sidebar_border)),
        _ => div()
            .flex()
            .items_center()
            .justify_center()
            .h(theme.metrics.control_small)
            .px_1()
            .child(
                div()
                    .w(RULE)
                    .h(theme.metrics.control_small / 2.)
                    .bg(theme.border),
            ),
    }
}

/// The round badge a tile shows under the pointer.
fn badge(glyph: &'static str, fill: Hsla, ink: Hsla) -> Div {
    div()
        .size(BADGE)
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(fill)
        .child(
            svg()
                .path(icons::path(glyph))
                .size(BADGE_ICON)
                .text_color(ink),
        )
}

/// The mark where a dragged tool would land: a bar across the sidebar, an upright one in a row.
fn marker(zone: Zone, cx: &App) -> AnyElement {
    let theme = *cx.theme();
    match zone {
        Zone::Sidebar => div()
            .flex()
            .items_center()
            .w_full()
            .h(theme.metrics.control_small / 2.)
            .px_2()
            .child(div().w_full().h(MARKER).rounded_full().bg(theme.primary))
            .into_any_element(),
        _ => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(theme.metrics.control_small)
            .h(theme.metrics.control_small)
            .child(
                div()
                    .w(MARKER)
                    .h(theme.metrics.control_small)
                    .rounded_full()
                    .bg(theme.primary),
            )
            .into_any_element(),
    }
}

/// The icon of `tool` at `size`, or the rule a separator draws.
fn glyph(tool: Tool, size: Pixels, cx: &App) -> AnyElement {
    let theme = *cx.theme();
    match tool {
        Tool::Separator => div()
            .w(RULE)
            .h(size)
            .bg(theme.muted_foreground)
            .into_any_element(),
        tool => svg()
            .path(icons::path(icon(tool)))
            .flex_none()
            .size(size)
            .text_color(theme.foreground)
            .into_any_element(),
    }
}

/// The palette over the page while the bars are arranged: what no bar holds, a way back to the
/// default arrangement and the button that ends arranging. A tool dropped on it leaves its bar.
pub(crate) fn palette(cx: &App) -> AnyElement {
    let theme = *cx.theme();
    let unplaced = Sonora::global(cx).settings.read(cx).panels().unplaced();
    let count = unplaced.len();
    let tiles = unplaced
        .into_iter()
        .enumerate()
        .map(|(index, tool)| offer(tool, index, count, cx));

    let card = div()
        .id("panels-palette")
        .role(Role::Dialog)
        .aria_label(t!("panels-title"))
        .aria_description(t!("panels-hint"))
        .occlude()
        .flex()
        .flex_col()
        .gap_4()
        .w(theme.metrics.cover * PALETTE_WIDTH)
        .max_w_full()
        .p_5()
        .rounded(ui::corners(theme.radius))
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .shadow_lg()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(theme.text(Text::Heading))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("panels-title")),
                )
                .child(
                    div()
                        .text_size(theme.text(Text::Body))
                        .text_color(theme.muted_foreground)
                        .child(t!("panels-hint")),
                ),
        )
        .child(
            div()
                .id("panels-offers")
                .role(Role::List)
                .aria_label(t!("panels-offers"))
                .grid()
                .grid_cols(COLUMNS)
                .gap_2()
                .children(tiles),
        )
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    Button::new("panels-restore")
                        .ghost()
                        .icon("icons/undo-2.svg")
                        .label(t!("panels-restore"))
                        .on_click(|_, _, cx| Customizing::restore(cx)),
                )
                .child(
                    Button::new("panels-done")
                        .primary()
                        .label(t!("panels-done"))
                        .on_click(|_, _, cx| Customizing::stop(cx)),
                ),
        );
    let scrim = div()
        .id("panels-scrim")
        .absolute()
        .inset_0()
        .bg(theme.overlay)
        .occlude();

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(theme.metrics.inset)
        .child(discards(scrim))
        .child(discards(card))
        .into_any_element()
}

/// Makes `base` take a tool dragged off its bar back to the palette: the palette itself and the
/// dimmed page around it. Both occlude, so each has to listen itself. Neither touches the aim
/// while a drag moves: the page runs on under a translucent player bar, a floating island and an
/// overlaid sidebar, and a bar already lets go of the aim when the pointer leaves it.
fn discards(base: Stateful<Div>) -> Stateful<Div> {
    base.on_drop(|dragged: &DraggedTool, _, cx| {
        Customizing::entity(cx).update(cx, |this, cx| {
            this.aim = None;
            this.held = None;
            cx.notify();
        });
        if let Some((zone, index)) = dragged.from {
            Customizing::remove(zone, index, cx);
        }
    })
}

/// One tool on the palette, which a drag takes into a bar and a click adds to the bar it
/// belongs in.
fn offer(tool: Tool, index: usize, count: usize, cx: &App) -> AnyElement {
    let theme = *cx.theme();
    let name = i18n::lookup(label(tool), None);
    let dragged = DraggedTool {
        tool,
        from: None,
        offset: Point::default(),
    };

    div()
        .id(SharedString::from(format!("panels-offer-{}", tool.id())))
        .role(Role::Button)
        .aria_label(t!("panels-add", name = name.as_ref()))
        .aria_position_in_set(index + 1)
        .aria_size_of_set(count)
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1p5()
        .h(theme.metrics.control * TILE_HEIGHT)
        .px_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .hover(|style| style.bg(theme.secondary_hover))
        .cursor_grab()
        .child(glyph(tool, theme.text(Text::Large), cx))
        .child(
            div()
                .w_full()
                .text_center()
                .line_clamp(2)
                .text_ellipsis()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(name),
        )
        .on_click(move |_, _, cx| Customizing::add(tool, cx))
        .on_drag(dragged, |dragged, offset, _, cx| {
            Customizing::hold(dragged.tool, None, cx);
            cx.new(|_| dragged.clone().at(offset))
        })
        .into_any_element()
}
