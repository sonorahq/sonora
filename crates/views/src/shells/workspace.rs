use std::time::Duration;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyView, App, Context, Div, Entity, FocusHandle, Pixels, Render, SpringConfig, StyleRefinement,
    px,
};
use gpui::{Window, deferred, div, linear_color_stop, linear_gradient};
use input::WORKSPACE_CONTEXT;
use state::{AppSettings, Playback, Queue, SideTab, Sonora};
use ui::{
    Activate, ActiveTheme as _, Deselect, Entrance, Remove, SelectNext, SelectPrevious, Sprung,
    VeilEdge, entering, shown_listing, veil, veiled,
};

use crate::chrome::{
    AppNotice, Chrome, PlayerBar, SidebarLeft, SidebarRight, SwipeCue, TitleBarOptions, sheet_frame,
};
use crate::shared::menus::CardMenu;
use crate::shells::Shell;

/// How close in pixels a sliding sidebar has to come to its width to count as landed.
const SLIDE_REST: f32 = 0.5;
/// The spring a sidebar slides out and back on: quick and without overshoot.
const SLIDE: SpringConfig = ui::spring(0.2, 0.);
/// How close the island's shift between the sheet's centre and the window's has to come to its
/// end to count as landed.
const SHIFT_REST: f32 = 0.002;
/// How tall the band the page fades out through under the island stands, as a multiple of the
/// island's bar, and how far the page blurs at its foot.
const ISLAND_FADE: f32 = 1.2;
const ISLAND_BLUR: Pixels = px(6.);

pub(crate) struct Workspace {
    sidebar: Entity<SidebarLeft>,
    player_bar: Entity<PlayerBar>,
    settings: Entity<AppSettings>,
    sidebar_right: Entity<SidebarRight>,
    card_menu: Entity<CardMenu>,
    notice: Entity<AppNotice>,
    swipe_cue: Entity<SwipeCue>,
    content: AnyView,
    /// A screen's own header, floated over the top of the page and outside its transition.
    /// The page pads itself to start beneath it.
    header: Option<AnyView>,
    /// Whether the page is laid out cached. A page that redraws part of itself every frame is
    /// not, since a cached view that redraws builds every view in it again, and it caches its
    /// own parts instead.
    cached: bool,
    transition: Option<Entrance>,
    focus: FocusHandle,
    left_slide: Slide,
    right_slide: Slide,
    /// Where the island stands between the sheet's centre, at 0, and the window's, at 1. It
    /// springs there when the island stops or starts fitting on the sheet.
    island_shift: Option<Sprung>,
}

/// How far a sidebar beside the page has slid out. It springs only when the listener opens or
/// closes the sidebar and follows any other change of width at once, so an edge drag or a window
/// resize never sets it sliding.
#[derive(Default)]
struct Slide {
    spring: Option<Sprung>,
    /// The sidebar's count of opens and closes as last seen.
    slides: usize,
}

impl Slide {
    /// Steps towards `width` and answers how wide the sidebar shows this frame. `slides` is the
    /// sidebar's count of opens and closes, and a new one starts a slide. The first call lands at
    /// once, so the window opens with its sidebars already out.
    fn follow(&mut self, width: Pixels, slides: usize, window: &Window, cx: &App) -> Pixels {
        let target = width / px(1.);
        let spring = self.spring.get_or_insert_with(|| Sprung::at(target));
        if slides == self.slides && spring.resting() {
            spring.jump(target);
        }
        self.slides = slides;
        px(spring.follow(target, SLIDE, SLIDE_REST, window, cx).max(0.))
    }
}

impl Workspace {
    pub fn new(
        playback: Entity<Playback>,
        queue: Entity<Queue>,
        content: AnyView,
        cx: &mut Context<Self>,
    ) -> Self {
        let sidebar = cx.new(SidebarLeft::new);
        let sidebar_right = cx.new(|cx| SidebarRight::new(queue.clone(), playback.clone(), cx));
        let player_bar = cx.new(|cx| PlayerBar::new(playback.clone(), queue, cx));
        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        Self {
            sidebar,
            player_bar,
            settings,
            sidebar_right,
            card_menu: CardMenu::entity(cx),
            notice: cx.new(AppNotice::new),
            swipe_cue: cx.new(SwipeCue::new),
            content,
            header: None,
            cached: true,
            transition: None,
            focus: cx.focus_handle(),
            left_slide: Slide::default(),
            right_slide: Slide::default(),
            island_shift: None,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn toggle_sidebar(&self, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| sidebar.toggle(cx));
    }

    pub fn toggle_sidebar_right(&self, cx: &mut Context<Self>) {
        self.sidebar_right.update(cx, |panel, cx| panel.toggle(cx));
    }

    pub fn show_side(&self, tab: SideTab, cx: &mut Context<Self>) {
        self.sidebar_right
            .update(cx, |panel, cx| panel.show(tab, cx));
    }

    /// Opens the right sidebar on `tab` without closing it when it already shows there.
    pub fn reveal_side(&self, tab: SideTab, cx: &mut Context<Self>) {
        self.sidebar_right
            .update(cx, |panel, cx| panel.reveal(tab, cx));
    }

    /// Shows a page, with the header it wants above it or none, and laid out `cached` or not.
    /// The three arrive together so a header can never outlive its screen.
    pub fn set_content(
        &mut self,
        content: AnyView,
        header: Option<AnyView>,
        cached: bool,
        cx: &mut Context<Self>,
    ) {
        self.content = content;
        self.header = header;
        self.cached = cached;
        cx.notify();
    }

    pub fn reveal_content(&mut self, cx: &mut Context<Self>) -> Duration {
        if cx.reduce_motion() {
            self.transition = None;
            return Duration::ZERO;
        }

        let entrance = Entrance::start();
        self.transition = Some(entrance);
        cx.notify();
        entrance.span()
    }

    pub fn finish_transition(&mut self, cx: &mut Context<Self>) {
        if self.transition.take().is_some() {
            cx.notify();
        }
    }

    fn hidden(&mut self, window: &mut Window, cx: &Context<Self>) -> f32 {
        if cx.reduce_motion() {
            self.transition = None;
            return 0.;
        }
        let Some(transition) = self.transition else {
            return 0.;
        };
        if transition.running() {
            window.request_animation_frame();
        }
        transition.hidden()
    }
}

impl Shell for Workspace {
    fn title_bar(&self, content: Option<AnyView>, cx: &App) -> TitleBarOptions {
        TitleBarOptions {
            navigation: true,
            sidebar_open: self.sidebar.read(cx).is_open(),
            sidebar_right: Some(self.sidebar_right.read(cx).is_open()),
            // The page lies on the chrome as a sheet with its own edge.
            border: false,
            sheet: true,
            content,
            transparent: false,
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let reserved = self.sidebar_right.read(cx).reserved_width();
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.adapt(reserved, window, cx));
        let overlay_width = self.sidebar.read(cx).overlay_width();
        // Each sidebar beside the page slides out and back, and the page gives way as it goes.
        let left_target = self.sidebar.read(cx).occupied_width();
        let right_target = self.sidebar_right.read(cx).occupied_width(window);
        let left_slides = self.sidebar.read(cx).slides();
        let right_slides = self.sidebar_right.read(cx).slides();
        let left = self.left_slide.follow(left_target, left_slides, window, cx);
        let right = self
            .right_slide
            .follow(right_target, right_slides, window, cx);
        let left_breadth = self.sidebar.read(cx).breadth();
        let right_breadth = self.sidebar_right.read(cx).breadth();
        // While a sidebar slides the page fills the sheet on every frame and is laid out again
        // at its width, as it is while the window is resized, so it never jumps or runs under a
        // sidebar.
        let place = move |this: Div| this.left(-overlay_width).right_0();
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.linger(left > Pixels::ZERO, cx));
        self.sidebar_right
            .update(cx, |sidebar, cx| sidebar.linger(right > Pixels::ZERO, cx));
        Chrome::publish(left, right, reserved, cx);
        let covered = self.sidebar_right.read(cx).covers_content(window);
        let overlay = self.sidebar.read(cx).overlays();
        let bar_height = PlayerBar::height(window, cx);
        let floating = PlayerBar::floating(window, cx);
        // A floating island has no bar to stop the page at, so it runs under it too.
        let underlap = self.settings.read(cx).player_bar_translucent() || floating;
        let bottom = match underlap {
            true => bar_height,
            false => Pixels::ZERO,
        };
        Chrome::set_bottom(bottom, cx);
        // The sheet meets the window's edge wherever no chrome lies beside it, rounding into a
        // sidebar as it slides out, and its bottom runs under a translucent player bar to the
        // window's foot. Pages pad against the corners the sheet lands with, and only the frame
        // drawn here rounds along with a slide.
        let beside = |shown: Pixels, breadth: Pixels| match breadth > Pixels::ZERO {
            true => shown / breadth,
            false => 0.,
        };
        let left_beside = |shown: Pixels| match overlay_width > Pixels::ZERO {
            true => 1.,
            false => beside(shown, left_breadth),
        };
        Chrome::set_sheet(
            left_beside(left_target),
            beside(right_target, right_breadth),
            underlap,
            cx,
        );
        let (corners, edges) = Chrome::sheet_shape(
            left_beside(left),
            beside(right, right_breadth),
            underlap,
            cx,
        );
        // A cached view is laid out from the style given here and its own root
        // style is never consulted, so it can only be cached while it is in the
        // flow at a width this knows: an overlaid sidebar places itself, and a
        // closed one hides itself and takes no space at all. A sliding one keeps its
        // full width and shows only the part that has slid out, nothing on the frame a slide
        // starts from, and is cached on either side so a slide never rebuilds what it moves.
        let sidebar = match overlay || (left == Pixels::ZERO && left_target == Pixels::ZERO) {
            true => self.sidebar.clone().into_any_element(),
            false => slid(
                self.sidebar
                    .clone()
                    .cached(StyleRefinement::default().w(left_breadth).h_full())
                    .into_any_element(),
                left,
                left_breadth,
                true,
            ),
        };
        let sidebar_right = match right == Pixels::ZERO && right_target == Pixels::ZERO {
            true => self.sidebar_right.clone().into_any_element(),
            false => slid(
                match right < right_breadth {
                    true => self
                        .sidebar_right
                        .clone()
                        .cached(StyleRefinement::default().w(right_breadth).h_full())
                        .into_any_element(),
                    false => self.sidebar_right.clone().into_any_element(),
                },
                right,
                right_breadth,
                false,
            ),
        };
        // The header is cached at the height it last measured, so a page that redraws every
        // frame leaves it be. Until it has measured itself it is laid out as it is.
        let top = crate::screens::settings::Layout::of(window, cx);
        let header = self.header.clone().map(|header| match top.measured {
            true => header
                .cached(StyleRefinement::default().w_full().h(top.top))
                .into_any_element(),
            false => header.into_any_element(),
        });
        // An island stands over the sheet as it is this frame, sliding with it as a sidebar comes
        // and goes, and reaches over a sidebar only when the sheet is too narrow for it. A bar
        // runs along the window's whole foot.
        let island = floating.then(|| {
            let sheet = overlay_width + left;
            let width = window.viewport_size().width - sheet - right;
            let target = match PlayerBar::island_fits(width, window, cx) {
                true => 0.,
                false => 1.,
            };
            let shift = self
                .island_shift
                .get_or_insert_with(|| Sprung::at(target))
                .follow(target, SLIDE, SHIFT_REST, window, cx);
            PlayerBar::island_span(sheet, width, shift, window, cx)
        });
        // While the island stands on the sheet clear of the sidebars, their floating buttons
        // stand at their foot rather than above a bar that is not there.
        let viewport = window.viewport_size().width;
        let (start, end) = (overlay_width + left, viewport - right);
        let over = island.is_some_and(|(at, width)| at < start || at + width > end);
        Chrome::set_island_clear(island.is_some() && !over, cx);
        let hidden = self.hidden(window, cx);
        // The scrim is what fades the outgoing page: it is the page's own colour at
        // full strength, so covering the content with it costs no layout and the
        // content view keeps its cache. A see-through window has no such colour. The
        // scrim would be one more translucent layer over the one the root already
        // paints, and the content area would sit visibly darker than the chrome
        // around it for the length of the transition — a quad can only add coverage,
        // never replace it. There the content carries the fade itself, which is the
        // same curve at the price of leaving the cached layout path while it runs.
        let dissolving = cx.theme().transparent;
        let scrim = match dissolving {
            true => 0.,
            false => hidden,
        };
        let theme = *cx.theme();
        let backdrop = theme.background;
        // That price has to be paid for real, not merely accepted: opacity is baked
        // into a primitive as it is painted, and a cached view replays its primitives
        // until something notifies it, so a fading page that stayed cached would be
        // recorded on the first frame, nearly clear, and replayed that way long after
        // the transition had ended. While it carries its own fade the content is
        // rendered uncached. Every screen sizes its root `size_full`, so the layout
        // is the one the cache would have used, and only the frames of the
        // transition pay for the render.
        // A sliding sidebar renders the page uncached too. A cached page would only learn of
        // the width published above after this frame, and would be laid out again at the new
        // size with the widths it worked out for an earlier one.
        let sliding = left != left_target || right != right_target;
        let content = match (dissolving && hidden > 0.) || !self.cached || sliding {
            true => self.content.clone().into_any_element(),
            false => self
                .content
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element(),
        };

        div()
            .id("workspace")
            .role(Role::Main)
            .relative()
            .flex()
            .flex_col()
            .w_full()
            .flex_1()
            .min_h_0()
            .key_context(super::shell_context(WORKSPACE_CONTEXT))
            .track_focus(&self.focus)
            .on_action(|_: &SelectNext, window, cx| {
                if let Some(table) = shown_listing(cx) {
                    table.select_next(window, cx);
                    cx.stop_propagation();
                }
            })
            .on_action(|_: &SelectPrevious, window, cx| {
                if let Some(table) = shown_listing(cx) {
                    table.select_previous(window, cx);
                    cx.stop_propagation();
                }
            })
            .on_action(|_: &Deselect, _, cx| {
                if let Some(table) = shown_listing(cx) {
                    table.deselect(cx);
                    cx.stop_propagation();
                }
            })
            .on_action(|_: &Activate, _, cx| {
                if let Some(table) = shown_listing(cx) {
                    table.activate(cx);
                    cx.stop_propagation();
                }
            })
            .on_action(|_: &Remove, _, cx| {
                if let Some(table) = shown_listing(cx) {
                    table.remove(cx);
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    // Under an opaque window the chrome backs the whole row. A see-through one
                    // stacks the alpha of every fill laid over another, so there each sidebar and
                    // the sheet paint only their own part of it.
                    .when(!theme.transparent, |this| this.bg(theme.sidebar))
                    .when(!overlay, |this| this.child(sidebar))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .ml(overlay_width)
                            // The page lies on the chrome as a sheet. Its fill stops at the
                            // curve of each rounded corner, where the sheet's frame lays the
                            // chrome's colour instead.
                            .bg(theme.background)
                            .rounded_tl(corners.top_left)
                            .rounded_tr(corners.top_right)
                            .rounded_bl(corners.bottom_left)
                            .rounded_br(corners.bottom_right)
                            .when(overlay, |this| this.overflow_hidden())
                            .when(covered, |this| this.hidden())
                            // A two-finger swipe across the page steps through its history,
                            // as in a browser.
                            .on_scroll({
                                let cue = self.swipe_cue.clone();
                                move |event, window, cx| {
                                    cue.update(cx, |cue, cx| cue.follow(event, window, cx));
                                }
                            })
                            .child(
                                div()
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_h_0()
                                    .when(hidden > 0., |this| this.overflow_hidden())
                                    .child(
                                        div()
                                            .absolute()
                                            .map(place)
                                            .top_0()
                                            .bottom_0()
                                            .flex()
                                            .flex_col()
                                            .map(|this| match dissolving {
                                                true => entering(this, hidden),
                                                false => veiled(this, hidden),
                                            })
                                            .child(content),
                                    )
                                    .when(scrim > 0., |this| {
                                        this.child(
                                            div()
                                                .absolute()
                                                .left_0()
                                                .right_0()
                                                .top_0()
                                                .bottom_0()
                                                .bg(backdrop)
                                                .opacity(scrim),
                                        )
                                    })
                                    // The header comes last, so it paints over the page and
                                    // the scrim, and spans the same width the page does
                                    // beneath an overlaid sidebar.
                                    .when_some(header, |this, header| {
                                        this.child(
                                            div().absolute().map(place).top_0().child(header),
                                        )
                                    }),
                            )
                            .when(floating, |this| {
                                this.child(island_fade(bar_height, window, cx))
                            })
                            .child(self.swipe_cue.clone())
                            .child(sheet_frame(corners, edges, cx)),
                    )
                    .child(sidebar_right)
                    .when(overlay, |this| this.child(self.sidebar.clone())),
            )
            .child(
                div()
                    .relative()
                    .map(|bar| match (island, underlap) {
                        (Some((left, width)), _) => bar.absolute().bottom_0().left(left).w(width),
                        (None, true) => bar.absolute().left_0().right_0().bottom_0(),
                        (None, false) => bar,
                    })
                    .child(
                        self.player_bar
                            .clone()
                            .cached(StyleRefinement::default().w_full().h(bar_height)),
                    ),
            )
            // Deferred so the cards sit above the sidebars' resize grips. The root draws the
            // dialogs after the shell, so those still cover them.
            .child(deferred(self.notice.clone()))
            .child(self.card_menu.clone())
    }
}

/// Shows `shown` of a sidebar `breadth` wide, cut along the side facing the page so it seems to
/// slide in from the window's edge, `leading` for the left one. A sidebar all the way out is laid
/// out as it is, so its edge grip can reach over the page.
fn slid(
    sidebar: gpui::AnyElement,
    shown: Pixels,
    breadth: Pixels,
    leading: bool,
) -> gpui::AnyElement {
    if shown >= breadth {
        return sidebar;
    }
    div()
        .relative()
        .flex_none()
        .h_full()
        .w(shown)
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(breadth)
                .map(|this| match leading {
                    true => this.right_0(),
                    false => this.left_0(),
                })
                .child(sidebar),
        )
        .into_any_element()
}

/// The band at the foot of the sheet that the island floats over. The page blurs and fades into
/// the sheet's colour towards the window's edge, so what scrolls under the island passes behind
/// it rather than through it. The part as tall as the island's bar takes no hover or click, so
/// nothing lights up around the pill, though the page still scrolls there.
fn island_fade(bar: Pixels, window: &Window, cx: &App) -> Div {
    let background = cx.theme().background;
    let band = bar * ISLAND_FADE;
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(band)
        .child(veil(
            VeilEdge::Bottom,
            band,
            ISLAND_BLUR,
            background,
            None,
            window,
        ))
        .child(div().absolute().inset_0().bg(linear_gradient(
            180.,
            linear_color_stop(background.opacity(0.), 0.),
            linear_color_stop(background, 1.),
        )))
        .child(
            div()
                .id("island-guard")
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(bar)
                .block_mouse_except_scroll(),
        )
}
