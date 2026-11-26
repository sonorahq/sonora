use gpui::prelude::*;
use gpui::{App, Context, Entity, Pixels, Render, StyleRefinement, Window, div, px};
use state::{AppSettings, Playback, Queue, SideTab, Sonora};
use ui::{ActiveTheme as _, Entrance, MIN_CONTENT, Panel, Room, Side, entering, veiled};

use crate::chrome::{Aside, Members};

const MIN_WIDTH: Pixels = px(240.);
const MAX_WIDTH: Pixels = px(560.);

fn fills_content(width: Pixels) -> bool {
    !Room::of(width).fits(Room::Wide)
}

/// The right sidebar: the queue and the lyrics, which share one view, and the members of the
/// session or Jam the listener is in.
pub(crate) struct SidebarRight {
    aside: Entity<Aside>,
    members: Entity<Members>,
    settings: Entity<AppSettings>,
    width: Pixels,
    open: bool,
    tab: SideTab,
    /// Whether the sidebar still slides away after closing, so it keeps drawing until it is gone.
    lingering: bool,
    /// How many times the listener has opened or closed the sidebar. The page springs aside only
    /// when this moves, so an edge drag or a window resize never sets the sidebar sliding.
    slides: usize,
    /// The entrance of the tab the open sidebar just switched to, the one a page makes as the
    /// listener moves to it, while it plays.
    entrance: Option<Entrance>,
}

impl SidebarRight {
    pub(crate) fn new(
        queue: Entity<Queue>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        let width = px(settings.read(cx).sidebar_right_width()).clamp(MIN_WIDTH, MAX_WIDTH);
        let open = settings.read(cx).sidebar_right_open();
        let tab = settings.read(cx).sidebar_right_tab();
        // the shared view starts on the queue while the members are what shows
        let shown = match tab {
            SideTab::Members => SideTab::Queue,
            tab => tab,
        };
        let aside = cx.new(|cx| Aside::new(queue, playback, shown, cx));
        let members = cx.new(Members::new);
        cx.observe(&settings, |this, settings, cx| {
            if this.tab != SideTab::Lyrics || settings.read(cx).lyrics_enabled() {
                return;
            }
            this.tab = SideTab::Queue;
            if this.open {
                this.enter(cx);
            }
            this.aside
                .update(cx, |aside, cx| aside.show(SideTab::Queue, cx));
            settings.update(cx, |settings, cx| {
                settings.set_sidebar_right_tab(SideTab::Queue, cx)
            });
            cx.notify();
        })
        .detach();

        Self {
            aside,
            members,
            settings,
            width,
            open,
            tab,
            lingering: false,
            slides: 0,
            entrance: None,
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn slides(&self) -> usize {
        self.slides
    }

    /// How wide the sidebar is when it shows, whether or not it shows now.
    pub(crate) fn breadth(&self) -> Pixels {
        self.width
    }

    /// Keeps a closed sidebar drawn while it slides away beside the page.
    pub(crate) fn linger(&mut self, lingering: bool, cx: &mut Context<Self>) {
        if self.lingering != lingering {
            self.lingering = lingering;
            cx.notify();
        }
    }

    pub(crate) fn available(window: &Window) -> bool {
        !fills_content(window.viewport_size().width)
    }

    pub(crate) fn covers_content(&self, _window: &Window) -> bool {
        false
    }

    pub(crate) fn occupied_width(&self, window: &Window) -> Pixels {
        match self.open && Self::available(window) {
            false => Pixels::ZERO,
            true => self.width,
        }
    }

    /// The width the left sidebar has to leave free for this one. It counts an open sidebar
    /// even while the window is too narrow to show it, so the left sidebar collapses first and
    /// stays collapsed when this one hides in turn.
    pub(crate) fn reserved_width(&self) -> Pixels {
        match self.open {
            false => Pixels::ZERO,
            true => self.width,
        }
    }

    pub(crate) fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        self.slides += 1;
        if self.open && self.tab != SideTab::Members {
            let tab = self.tab;
            self.aside.update(cx, |aside, cx| aside.show(tab, cx));
        }
        self.remember(cx);
        cx.notify();
    }

    pub(crate) fn show(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        if tab == SideTab::Lyrics && !self.settings.read(cx).lyrics_enabled() {
            return;
        }
        if self.open && self.tab == tab {
            self.close(cx);
            return;
        }
        if self.tab != tab {
            self.settings
                .update(cx, |settings, cx| settings.set_sidebar_right_tab(tab, cx));
        }
        if tab != SideTab::Members {
            self.aside.update(cx, |aside, cx| aside.show(tab, cx));
        }
        // A sidebar that opens on the tab slides out with it instead.
        if self.open && self.tab != tab {
            self.enter(cx);
        }
        self.tab = tab;
        if !self.open {
            self.slides += 1;
        }
        self.open = true;
        self.remember(cx);
        cx.notify();
    }

    /// Shows `tab`, and leaves the sidebar open when it already shows it, where `show` would
    /// close it.
    pub(crate) fn reveal(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        if self.open && self.tab == tab {
            return;
        }
        self.show(tab, cx);
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.aside.update(cx, |aside, cx| aside.dismiss(cx));
        if self.open {
            self.slides += 1;
        }
        self.open = false;
        self.remember(cx);
        cx.notify();
    }

    /// Starts the entrance over the tab just switched to, unless the listener reduces motion.
    fn enter(&mut self, cx: &App) {
        self.entrance = (!cx.reduce_motion()).then(Entrance::start);
    }

    /// How much of the tab just switched to is still hidden, from 1 as the switch starts to 0
    /// once its entrance ends. It asks for another frame while the entrance runs.
    fn hidden(&mut self, window: &mut Window, cx: &App) -> f32 {
        let Some(entrance) = self.entrance else {
            return 0.;
        };
        if cx.reduce_motion() || !entrance.running() {
            self.entrance = None;
            return 0.;
        }
        window.request_animation_frame();
        entrance.hidden()
    }

    fn remember(&self, cx: &mut Context<Self>) {
        let open = self.open;
        self.settings
            .update(cx, |settings, cx| settings.set_sidebar_right_open(open, cx));
    }

    fn persist(&self, cx: &mut Context<Self>) {
        let width = self.width / px(1.);
        self.settings.update(cx, |settings, cx| {
            settings.set_sidebar_right_width(width, cx)
        });
    }
}

impl Render for SidebarRight {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !(self.open && Self::available(window)) && !self.lingering {
            return div().into_any_element();
        }

        let theme = *cx.theme();
        // The tab switched to enters the way a page does: a scrim of the sidebar's own colour
        // fades off it, so the queue and lyrics stay cached. A see-through window has no such
        // colour, so there the tab carries the fade itself and is drawn uncached while it runs,
        // since a cached view replays the opacity it was first painted with.
        let hidden = self.hidden(window, cx);
        let dissolving = theme.transparent && hidden > 0.;
        let tab = match self.tab {
            SideTab::Members => self.members.clone().into_any_element(),
            _ if dissolving => self.aside.clone().into_any_element(),
            _ => self
                .aside
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element(),
        };

        Panel::new("sidebar-right", Side::Right, self.width)
            .limits(MIN_WIDTH, MAX_WIDTH)
            .reach(super::cap(MIN_WIDTH, MAX_WIDTH, MIN_CONTENT, window))
            .grip_above(super::Chrome::bottom(cx))
            .on_resize(cx.listener(|this, width: &Pixels, _, cx| {
                this.width = *width;
                this.persist(cx);
                cx.notify();
            }))
            // It lies on the chrome beside the page's sheet, which draws the only edge between
            // them.
            .bg(theme.sidebar)
            .border_color(gpui::transparent_black())
            .child(
                div()
                    .relative()
                    .size_full()
                    .min_h_0()
                    .when(hidden > 0., |this| this.overflow_hidden())
                    .child(
                        div()
                            .size_full()
                            // At rest the tab is drawn bare, outside any layer, so whatever
                            // frosts in it keeps its backdrop.
                            .when(hidden > 0., |this| match theme.transparent {
                                true => entering(this, hidden),
                                false => veiled(this, hidden),
                            })
                            .child(tab),
                    )
                    .when(hidden > 0. && !theme.transparent, |this| {
                        this.child(div().absolute().inset_0().bg(theme.sidebar).opacity(hidden))
                    }),
            )
            .into_any_element()
    }
}
