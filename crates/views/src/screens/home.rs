//! The home page: the Quick picks deck over the provider's shelves (Recently added, Artists, the
//! genre rails), with their loading skeletons, and the No connection page in their place when
//! the feed cannot be reached.

use std::rc::Rc;

use i18n::t;
use music::GenreItem;
use router::Destination;
use rune::{Cx, El, PointerPress, ScrollSpec, Tx, col};

use crate::Ui;
use crate::chrome::player_bar;
use crate::shared::cards::OnItem;
use crate::shared::menus::{self, Target};
use crate::shared::page::{self, rems};
use crate::shared::picks::{Picks, column_count};
use crate::shared::shelves::{Rails, Shelves};
use crate::shared::trouble;

/// The space between the deck and the shelves, in rems.
const STACK_GAP: f32 = 2.;

/// The home page's own view state.
#[derive(Default)]
pub struct State {
    /// The Quick picks page on show, with the column count it was turned at: a deck that
    /// packs a different number of columns starts again from its first page.
    picks: (usize, usize),
    /// The sideways scroll of every shelf.
    rails: Rails,
}

/// Tells the feed whether the home page is the one on screen, so it lands changes to what is
/// drawn only while nobody is looking. Called as the window arrives anywhere.
pub(crate) fn arrived(ui: &mut Ui, destination: &Destination) {
    ui.store
        .set_home_visible(matches!(destination, Destination::Home));
}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    if let Some(failure) = failure(ui) {
        return col()
            .key("home-failure")
            .role("alert")
            .w_full()
            .h_full()
            .child(failure);
    }
    let theme = &ui.theme;
    let inset = theme.metrics.inset;
    let width = page::content_width(cx, inset * 2.);
    let home = &ui.store.home;

    let items = home.quick_picks();
    let columns = column_count(width);
    let page = match ui.screens.home.picks {
        (turned, page) if turned == columns => page,
        _ => 0,
    };
    let picks = Picks::mixed("quick-picks", &items, width, page)
        .title("home-quick-picks")
        .vacancy("home-quick-picks-empty")
        .loading(home.is_loading(&ui.store.session, &ui.store.library))
        .on_page(move |ui: &mut Ui, page| {
            ui.screens.home.picks = (columns, page);
            menus::close(ui);
        })
        .on_item(item_menu())
        .build(ui);

    let sections = home.sections();
    let shelves = match sections.is_empty() && home.is_feeding() {
        true => Shelves::pending("home-shelf", width, theme),
        false => Shelves::new("home-shelf", &sections, &ui.screens.home.rails, width)
            .on_item(item_menu())
            .build(ui),
    };

    col()
        .key("home-page")
        .w_full()
        .h_full()
        .min_h(0.)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("home-content")
                .w_full()
                .padding(inset)
                .pb(inset + player_bar::bottom(ui, cx))
                .gap(rems(theme, STACK_GAP))
                .child(picks)
                .child(shelves),
        )
}

/// The page home shows in place of its shelves: the No connection state the moment the network
/// is gone, since every row here comes from the provider, and the reason its feed failed
/// otherwise. Asking for the feed again is the retry, and it starts the pauses between tries
/// over.
fn failure(ui: &Ui) -> Option<El> {
    let reason = match ui.store.network.lost() {
        true => None,
        false => Some(ui.store.home.error()?.to_owned()),
    };
    Some(trouble::lost(
        ui,
        "home-lost",
        t!("trouble-not-loaded"),
        reason.as_deref(),
        |ui: &mut Ui| ui.store.retry_home(),
    ))
}

/// Opens the context menu of a shelf item where the pointer pressed.
fn item_menu() -> OnItem {
    Rc::new(
        |ui: &mut Ui, item: GenreItem, press: PointerPress, _: &mut Tx| {
            if let Some(target) = Target::of(&item) {
                menus::open(ui, target, press);
            }
        },
    )
}
