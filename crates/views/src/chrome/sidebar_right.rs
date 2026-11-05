use gpui::prelude::*;
use gpui::{Context, Entity, Pixels, Render, StyleRefinement, Window, div, px};
use state::{AppSettings, Playback, Queue, SideTab, Sonora};
use ui::{ActiveTheme as _, MIN_CONTENT, Panel, Room, Side};

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

        Self {
            aside,
            members,
            settings,
            width,
            open,
            tab,
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
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
        if self.open && self.tab != SideTab::Members {
            let tab = self.tab;
            self.aside.update(cx, |aside, cx| aside.show(tab, cx));
        }
        self.remember(cx);
        cx.notify();
    }

    pub(crate) fn show(&mut self, tab: SideTab, cx: &mut Context<Self>) {
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
        self.tab = tab;
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
        self.open = false;
        self.remember(cx);
        cx.notify();
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
        if !self.open || !Self::available(window) {
            return div().into_any_element();
        }

        let theme = *cx.theme();

        Panel::new("sidebar-right", Side::Right, self.width)
            .limits(MIN_WIDTH, MAX_WIDTH)
            .reach(super::cap(MIN_WIDTH, MAX_WIDTH, MIN_CONTENT, window))
            .grip_above(super::Chrome::bottom(cx))
            .on_resize(cx.listener(|this, width: &Pixels, _, cx| {
                this.width = *width;
                this.persist(cx);
                cx.notify();
            }))
            .when(!theme.transparent, |this| this.bg(theme.background))
            .border_color(theme.border)
            .child(match self.tab {
                SideTab::Members => self.members.clone().into_any_element(),
                _ => self
                    .aside
                    .clone()
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element(),
            })
            .into_any_element()
    }
}
