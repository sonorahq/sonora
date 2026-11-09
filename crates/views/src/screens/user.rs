//! A user's profile: the hero with their picture, followers and following, over a grid of their
//! public playlists.

use std::borrow::Cow;
use std::rc::Rc;
use ui::Smooth as _;

use i18n::t;
use music::{GenreItem, Playlist};
use rune::{Cx, El, PointerPress, ScrollSpec, Tx, col, row};
use state::Profile;
use ui::Card;

use crate::Ui;
use crate::chrome::player_bar;
use crate::shared::album_grid::{CardGrid, CardLayout, card_row};
use crate::shared::cards::{self, Fit, OnItem};
use crate::shared::hero::{Hero, MetaStrip};
use crate::shared::menus::{self, Target};
use crate::shared::page::{self, rems};
use crate::shared::{cells, trouble};

const FALLBACK: &str = "icons/user.svg";
/// How many skeleton cards stand in while the playlists load.
const PENDING: usize = 6;
/// The room between the hero and the playlists, and under the playlists' heading, in rems.
const SECTION_GAP: f32 = 2.;
const HEADING_GAP: f32 = 1.;
/// The room between rows of skeleton cards, as the grid spaces its rows, in rems.
const ROW_GAP: f32 = 1.5;

/// The profile page's own state. Everything it shows lives in the store's profile page.
#[derive(Default)]
pub struct State {}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let profile = ui.store.pages.user.as_ref();
    if let Some(failure) = profile.and_then(|profile| failure(ui, profile)) {
        return col()
            .key("user-failure")
            .role("alert")
            .w_full()
            .h_full()
            .center()
            .child(failure);
    }
    let inset = theme.metrics.inset;
    let available = page::content_width(cx, 0.) - page::reserved(inset);
    let bottom = inset + player_bar::bottom(ui, cx);
    let loading = profile.is_some_and(Profile::is_loading);
    let listed = profile.map(Profile::playlists).unwrap_or_default();

    let heading = ui::spoken_heading("user-playlists-heading", t!("user-playlists"), 2, theme);
    let page = col()
        .key("user-page")
        .w_full()
        .h_full()
        .min_h(0.)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .smooth::<Ui>("user-page")
        .child(hero(ui, profile))
        .child(
            row()
                .key("user-playlists-head")
                .w_full()
                .px(inset)
                .pt(rems(theme, SECTION_GAP))
                .pb(rems(theme, HEADING_GAP))
                .child(heading),
        );
    if loading {
        let layout = CardLayout::new(available);
        return page.child(
            col()
                .key("user-pending")
                .w_full()
                .px(inset)
                .pb(bottom)
                .gap(rems(theme, ROW_GAP))
                .children((0..PENDING).step_by(layout.columns.max(1)).map(|first| {
                    card_row(
                        layout,
                        (first..(first + layout.columns).min(PENDING)).map(|place| {
                            Card::skeleton(format!("user-pending-{place}"))
                                .tile(layout.card)
                                .build(theme)
                        }),
                    )
                }))
                .a11y_hidden(),
        );
    }
    if listed.is_empty() {
        return page.child(
            col()
                .key("user-playlists-empty-frame")
                .w_full()
                .px(inset)
                .pb(bottom)
                .child(
                    ui::spoken_vacant("user-playlists-empty", t!("user-playlists-empty"), theme)
                        .role("status"),
                ),
        );
    }
    let opener: OnItem = Rc::new(
        |ui: &mut Ui, item: GenreItem, press: PointerPress, _: &mut Tx| {
            if let Some(target) = Target::of(&item) {
                menus::open(ui, target, press);
            }
        },
    );
    page.child(
        CardGrid::new("user-playlists", available, listed.len())
            .inset(inset)
            .bottom(bottom)
            .build(
                theme,
                |ui: &Ui, at: usize| {
                    playlists(ui)
                        .get(at)
                        .map_or(Cow::Borrowed(""), |playlist| Cow::Borrowed(&playlist.id))
                },
                move |ui: &Ui, at: usize, width: f32| {
                    let Some(playlist) = playlists(ui).get(at) else {
                        return Card::skeleton(format!("user-playlist-{at}"))
                            .tile(width)
                            .build(&ui.theme);
                    };
                    cards::playlist_card(
                        ui,
                        format!("user-playlist-{}", playlist.id),
                        playlist,
                        Fit::Tile(width),
                        Some(&opener),
                    )
                    .flat()
                    .build(&ui.theme)
                },
            ),
    )
}

fn playlists(ui: &Ui) -> &[Playlist] {
    ui.store
        .pages
        .user
        .as_ref()
        .map(Profile::playlists)
        .unwrap_or_default()
}

/// The hero: the round picture, the name (or the id while it is on its way), and the followers
/// and following counts.
fn hero(ui: &Ui, profile: Option<&Profile>) -> El {
    let theme = &ui.theme;
    let user = profile.and_then(Profile::user);
    let title = user
        .map(|user| user.name.clone())
        .or_else(|| profile.and_then(Profile::id).map(str::to_owned))
        .unwrap_or_default();
    let mut strip = MetaStrip::new();
    if let Some(followers) = user.and_then(|user| user.followers) {
        let value = cells::count(followers);
        strip = strip.text(t!("user-followers", count = followers, value = &value).into_owned());
    }
    if let Some(following) = user.and_then(|user| user.following) {
        let value = cells::count(following);
        strip = strip.text(t!("user-following", count = following, value = &value).into_owned());
    }
    Hero::new("user-hero", title)
        .pending(user.is_none() && profile.is_some_and(Profile::is_loading))
        .cover(user.and_then(|user| user.avatar.clone()))
        .circle()
        .fallback(FALLBACK)
        .eyebrow(t!("user-eyebrow").into_owned())
        .meta(strip.build(theme))
        .padded()
        .build(ui)
}

/// The page a profile that did not load shows instead of its hero and playlists. Opening the same
/// profile again is the retry, since a failed load leaves the page empty.
fn failure(ui: &Ui, profile: &Profile) -> Option<El> {
    let id = profile.id()?.to_owned();
    let reason = match trouble::unreachable(ui, &id) {
        true => None,
        false => Some(profile.error()?.to_owned()),
    };
    Some(trouble::lost(
        ui,
        "user-lost",
        t!("trouble-not-loaded"),
        reason.as_deref(),
        move |ui: &mut Ui| ui.store.open_user(&id),
    ))
}
