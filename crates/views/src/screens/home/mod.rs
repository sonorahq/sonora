use std::rc::Rc;

use crate::chrome::Chrome;
use crate::shared::menus::{Item, ItemMenu};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, Entity, MouseDownEvent, Pixels, Point, Render, ScrollHandle, WeakEntity,
    Window, div, px,
};
use i18n::t;
use music::GenreItem;
use state::{Home, Network, Playback, SessionState, Sonora};
use ui::{ActiveTheme as _, Mode, Popup, Scrollbar, Scroller};

use crate::shared::cells;
use crate::shared::picks::{Picks, Shape};
use crate::shared::shelves::Shelves;
use crate::shared::tracks::{PlaybackStatus, playback_status};
use crate::shared::trouble;

const STEADY: Pixels = px(0.5);

pub(crate) struct HomeView {
    home: Entity<Home>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    shelves: Entity<Shelves>,
    width: Pixels,
    quick_picks: Pager,
    scrollbar: Entity<Scrollbar>,
    /// The submenu state of a track's context menu.
    menus: ItemMenu,
    context_menu: Option<(Item, Point<Pixels>)>,
}

#[derive(Default)]
struct Pager {
    columns: usize,
    page: usize,
}

impl Pager {
    fn fit(&mut self, columns: usize, pages: usize) -> usize {
        if self.columns != columns {
            self.columns = columns;
            self.page = 0;
        }
        self.page = self.page.min(pages.saturating_sub(1));
        self.page
    }
}

impl HomeView {
    pub(crate) fn new(
        home: Entity<Home>,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let me = cx.entity_id();
        let playlist_scrollbar = cx.new(|_| Scrollbar::inset().watching(me));
        let menus = ItemMenu::new(playlist_scrollbar, cx);

        cx.observe(&home, |this, _, cx| {
            this.menus.reset(cx);
            this.context_menu = None;
            cx.notify();
        })
        .detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();

        let library = Sonora::global(cx).library.clone();
        cx.observe(&library, |_, _, cx| cx.notify()).detach();

        let shelves = cx.new(|_| Shelves::new("home-shelf", me, playback.clone()));
        cx.observe(&shelves, |_, _, cx| cx.notify()).detach();

        let current_playback = playback_status(&playback, cx);
        cx.observe(&playback, |this, playback, cx| {
            let current = playback_status(&playback, cx);
            if this.playback_status != current {
                this.playback_status = current;
                cx.notify();
            }
        })
        .detach();

        Self {
            home,
            playback,
            playback_status: current_playback,
            shelves,
            width: Pixels::ZERO,
            quick_picks: Pager::default(),
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(me)),
            menus,
            context_menu: None,
        }
    }

    /// Tells the feed whether this page is the one on screen, so it lands changes to what is
    /// drawn only while nobody is looking.
    pub(crate) fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.home
            .update(cx, |home, cx| home.set_visible(visible, cx));
    }

    /// The Quick picks deck, headed by the name of the account it was drawn from. It is the
    /// one deck on the page: what the provider lists as played lately and its picks after
    /// them, a skeleton while they are on their way.
    fn quick_picks(&mut self, width: Pixels, cx: &mut Context<Self>) -> AnyElement {
        let items = self.home.read(cx).quick_picks();
        let shape = Shape::new(width, items.len());
        let pages = shape.pages;
        let page = self.quick_picks.fit(shape.columns, shape.pages);
        let name = match self.home.read(cx).is_local(cx) {
            true => None,
            false => match Sonora::global(cx).session.read(cx).state() {
                SessionState::SignedIn(profile) => Some(profile.display_name.clone()),
                _ => None,
            },
        };
        let opened = items.clone();
        let home = cx.entity().downgrade();
        let refreshed = self.home.clone();

        Picks::mixed("quick-picks", items, self.playback.clone(), width, page)
            .title("home-quick-picks")
            .vacancy("home-quick-picks-empty")
            .loading(self.home.read(cx).is_loading(cx))
            .refresh_disabled(self.home.read(cx).is_feeding())
            .on_refresh(move |_, _, cx| {
                refreshed.update(cx, |home, cx| home.refresh_picks(cx));
            })
            .when_some(name.filter(|name| !name.is_empty()), |deck, name| {
                deck.eyebrow(name)
            })
            .on_previous(cx.listener(|this, _, _, cx| {
                this.quick_picks.page = this.quick_picks.page.saturating_sub(1);
                this.context_menu = None;
                cx.notify();
            }))
            .on_next(cx.listener(move |this, _, _, cx| {
                this.quick_picks.page = (this.quick_picks.page + 1).min(pages.saturating_sub(1));
                this.context_menu = None;
                cx.notify();
            }))
            .on_context_menu(move |place, event, _, cx| {
                open_item_menu(&home, &opened, place, event, cx);
            })
            .into_any_element()
    }

    /// The page home shows in place of its shelves: the No connection state the moment the
    /// network is gone, since every row here comes from the provider, and the reason its feed
    /// failed otherwise. Asking for the feed again is the retry, and it starts the pauses
    /// between tries over.
    fn failure(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let reason = match Network::lost(cx) {
            true => None,
            false => Some(self.home.read(cx).error()?.to_owned()),
        };
        let home = self.home.clone();

        Some(
            trouble::lost(
                "home-lost",
                t!("trouble-not-loaded"),
                reason.as_deref(),
                move |_, _, cx| {
                    home.update(cx, |home, cx| home.retry(cx));
                },
            )
            .size_full()
            .into_any_element(),
        )
    }
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(failure) = self.failure(cx) {
            return div().size_full().child(failure).into_any_element();
        }

        let theme = *cx.theme();
        let available = cells::content_width(window, theme.metrics.inset * 2., cx);
        if (available - self.width).abs() >= STEADY {
            self.width = available;
        }
        let context_menu = self.context_menu.clone().map(|(item, position)| {
            let menu = item.menu(&self.menus, self.playback.clone(), false, cx);
            Popup::new(position, menu).on_close(cx.listener(|this, _, _, cx| {
                this.context_menu = None;
                cx.notify();
            }))
        });

        let picks = self.quick_picks(available, cx);
        let sections = self.home.read(cx).sections();
        let loading_sections = self.home.read(cx).is_loading_sections();
        let width = self.width;
        let shelves = self
            .shelves
            .update(cx, |shelves, cx| match loading_sections {
                true => shelves.pending(width, cx),
                false => vec![shelves.render(sections, Mode::Grid, width, window, cx)],
            });

        Scroller::new("home-page", &self.scrollbar)
            .p(theme.metrics.inset)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_8()
                    .child(picks)
                    .children(shelves),
            )
            .when_some(context_menu, |this, menu| this.child(menu))
            .into_any_element()
    }
}

fn open_item_menu(
    home: &WeakEntity<HomeView>,
    items: &Rc<Vec<GenreItem>>,
    place: usize,
    event: &MouseDownEvent,
    cx: &mut gpui::App,
) {
    let Some(item) = items.get(place).and_then(Item::of) else {
        return;
    };
    let Some(home) = home.upgrade() else {
        return;
    };
    home.update(cx, |this, cx| {
        this.menus.reset(cx);
        this.context_menu = Some((item, event.position));
        cx.notify();
    });
}
