use gpui::prelude::*;
use gpui::{Context, Entity, Pixels, Render, StyleRefinement, Window, div, px};
use state::{AppSettings, Playback, Queue, SideTab, Sonora};
use ui::{ActiveTheme as _, MIN_CONTENT, Motion, Panel, Room, Side, Transition, slide, snapped};

use crate::chrome::Aside;

const MIN_WIDTH: Pixels = px(240.);
const MAX_WIDTH: Pixels = px(560.);

fn fills_content(width: Pixels) -> bool {
    !Room::of(width).fits(Room::Wide)
}

pub(crate) struct SidebarRight {
    aside: Entity<Aside>,
    settings: Entity<AppSettings>,
    width: Pixels,
    open: bool,
    transition: Option<Transition>,
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
        let aside = cx.new(|cx| Aside::new(queue, playback, tab, cx));

        Self {
            aside,
            settings,
            width,
            open,
            transition: None,
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
        if !Self::available(window) || !self.open {
            return Pixels::ZERO;
        }
        snapped(self.width, window)
    }

    pub(crate) fn toggle(&mut self, cx: &mut Context<Self>) {
        self.set_open(!self.open, cx);
    }

    pub(crate) fn show(&mut self, tab: SideTab, cx: &mut Context<Self>) {
        if self.open && self.aside.read(cx).tab() == tab {
            self.close(cx);
            return;
        }
        if self.aside.read(cx).tab() != tab {
            self.settings
                .update(cx, |settings, cx| settings.set_sidebar_right_tab(tab, cx));
        }
        self.aside.update(cx, |aside, cx| aside.show(tab, cx));
        if !self.open {
            self.set_open(true, cx);
        } else {
            self.remember(cx);
            cx.notify();
        }
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.aside.update(cx, |aside, cx| aside.dismiss(cx));
        self.set_open(false, cx);
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if self.open {
            let tab = self.aside.read(cx).tab();
            self.aside.update(cx, |aside, cx| aside.show(tab, cx));
        }
        self.start_transition(cx);
        self.remember(cx);
        cx.notify();
    }

    fn start_transition(&mut self, cx: &Context<Self>) {
        if cx.reduce_motion() {
            self.transition = None;
            return;
        }

        let current = self.transition.map(|t| t.fraction()).unwrap_or(match self.open {
            true => 0.0,
            false => 1.0,
        });

        self.transition = Some(Transition::toggle(self.open, current, Motion::Base));
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

    fn current_fraction(&mut self, window: &mut Window, cx: &mut Context<Self>) -> f32 {
        let target = match self.open {
            true => 1.0,
            false => 0.0,
        };
        Transition::step(&mut self.transition, target, window, cx)
    }
}

impl Render for SidebarRight {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !Self::available(window) {
            return div().into_any_element();
        }

        let fraction = self.current_fraction(window, cx);
        if fraction <= 0.0 {
            return div().into_any_element();
        }

        let current_width = snapped(self.width * fraction, window);
        let theme = *cx.theme();

        let aside = match fraction < 1.0 {
            true => self.aside.clone().into_any_element(),
            false => self
                .aside
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element(),
        };

        let panel = Panel::new("sidebar-right", Side::Right, self.width)
            .limits(MIN_WIDTH, MAX_WIDTH)
            .reach(super::cap(MIN_WIDTH, MAX_WIDTH, MIN_CONTENT, window))
            .on_resize(cx.listener(|this, width: &Pixels, _, cx| {
                this.width = *width;
                this.persist(cx);
                cx.notify();
            }))
            .when(!theme.transparent, |this| this.bg(theme.background))
            .border_color(theme.border)
            .child(aside);

        if fraction >= 1.0 {
            panel.into_any_element()
        } else {
            slide(Side::Right, current_width, self.width, panel).into_any_element()
        }
    }
}
