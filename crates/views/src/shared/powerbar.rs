use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Global, MouseButton, Pixels, Render,
    ScrollHandle, SharedString, SpringConfig, Window, anchored, deferred, div, point, px,
};
use i18n::t;
use input::{POWERBAR_CONTEXT, PowerbarDetails};
use router::{Destination, navigate};
use state::{Hit, Origin, Playback, Search};
use ui::{
    ActiveTheme as _, Card, Cascade, Dismiss, Input, Kbd, SelectNext, SelectPrevious, Spinner,
    Sprung, Submit, Text, Unfold, blurring, corners, effects, popover_fill, snapped, spoken_vacant,
};

use crate::screens::search::{noun, tag};
use crate::shared::tracks::{PlaybackStatus, playback_status};
use crate::shared::trouble;

/// How many results the powerbar lists, taken from the top of the ranked search.
const MAX_HITS: usize = 8;
/// How far down the room under the title bar the panel hangs, as a share of that room.
const PERCH: f32 = 0.14;
/// How wide the panel is, in covers.
const WIDTH: f32 = 4.6;
/// How tall the results may grow, as a share of the room under the title bar, before they scroll.
const REACH: f32 = 0.56;
/// The panel dropping into place. The little bounce makes it land rather than stop.
const ARRIVE: SpringConfig = ui::spring(0.3, 0.12);
/// The panel lifting away, quicker than it came and with no bounce.
const LEAVE: SpringConfig = ui::spring(0.16, 0.);
/// How far above its place the panel starts its drop.
const DROP: Pixels = px(28.);
/// How small the panel starts, as a share of its size.
const ZOOM: f32 = 0.92;
/// How hard the panel blurs as it starts to arrive.
const BLUR: Pixels = px(18.);
/// How hard the window behind the scrim blurs once the panel is open.
const SCRIM_BLUR: Pixels = px(12.);
/// How much of the page colour the scrim lays over the window.
const SCRIM: f32 = 0.42;
/// How hard the frost under the panel blurs what it covers.
const FROST: Pixels = px(24.);
/// Below this a blur is not worth a layer of its own.
const BLUR_LEAST: Pixels = px(0.05);
/// How close the panel's spring has to come to count as landed.
const REST: f32 = 0.001;

/// A floating quick search over the whole window. It drops in from above with the window
/// blurring away behind it, and its results surface one after another as they arrive.
pub(crate) struct Powerbar {
    open: bool,
    input: Entity<Input>,
    search: Entity<Search>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    hits: Vec<Hit>,
    selected: Option<usize>,
    restore: Option<FocusHandle>,
    scroll: ScrollHandle,
    /// How far the panel has arrived, 0 gone and 1 in place. The panel stays drawn while this
    /// springs back to 0 after a close.
    shown: Sprung,
    /// The results' box, stretching to fit as they come and go.
    unfold: Unfold,
    /// The results surfacing one after another as they arrive.
    cascade: Cascade,
}

struct Installed(Entity<Powerbar>);
impl Global for Installed {}

impl Powerbar {
    pub fn entity(
        search: Entity<Search>,
        playback: Entity<Playback>,
        cx: &mut App,
    ) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let bar = cx.new(|cx| {
                let current_status = playback_status(&playback, cx);

                let input = cx.new(|cx| {
                    Input::new("powerbar-placeholder", cx)
                        .name("powerbar-title")
                        .icon("icons/search.svg")
                        .bare()
                        .clearable()
                });
                cx.observe(&input, |this: &mut Powerbar, input, cx| {
                    let query = input.read(cx).text().to_owned();
                    // A new query starts the keyboard cursor over, while a caret move or a
                    // selection in the field leaves it where it was.
                    if this.search.read(cx).query() == query.trim() {
                        return;
                    }
                    this.selected = None;
                    this.scroll.set_offset(point(Pixels::ZERO, Pixels::ZERO));
                    this.search.update(cx, |search, cx| search.ask(&query, cx));
                    cx.notify();
                })
                .detach();

                cx.observe(&search, |this: &mut Powerbar, _, cx| {
                    this.rebuild_items(cx);
                })
                .detach();

                cx.observe(&playback, |this: &mut Powerbar, playback, cx| {
                    let current = playback_status(&playback, cx);
                    if this.playback_status != current {
                        this.playback_status = current;
                        cx.notify();
                    }
                })
                .detach();

                Self {
                    open: false,
                    input,
                    search,
                    playback,
                    playback_status: current_status,
                    hits: Vec::new(),
                    selected: None,
                    restore: None,
                    scroll: ScrollHandle::new(),
                    shown: Sprung::at(0.),
                    unfold: Unfold::default(),
                    cascade: Cascade::default(),
                }
            });
            cx.set_global(Installed(bar));
        }
        cx.global::<Installed>().0.clone()
    }

    pub fn toggle(window: &mut Window, cx: &mut App) {
        let Some(bar) = cx.try_global::<Installed>().map(|i| i.0.clone()) else {
            return;
        };
        bar.update(cx, |this, cx| {
            if this.open {
                this.close(window, cx);
            } else {
                this.show(window, cx);
            }
        });
    }

    /// Opens the panel on an empty query. The previous query is cleared here rather than on
    /// close, so the results stay put while the panel lifts away.
    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore = window.focused(cx);
        self.open = true;
        self.selected = None;
        // Results from the last time stay only for the lift away. Search may have been reset
        // since, sign-outs included, and an empty query does not notify.
        self.hits.clear();
        self.cascade.clear();
        self.unfold.reset();
        self.scroll.set_offset(point(Pixels::ZERO, Pixels::ZERO));
        self.input.update(cx, |input, cx| {
            input.set_text("", cx);
            input.focus(window, cx);
        });
        self.search.update(cx, |search, cx| search.ask("", cx));
        cx.notify();
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn rebuild_items(&mut self, cx: &mut Context<Self>) {
        // A closing panel keeps showing what it had, so an answer that lands as it lifts away
        // does not reshuffle it.
        if !self.open {
            return;
        }
        let search = self.search.read(cx);
        let hits: Vec<Hit> = match search.query().trim().is_empty() {
            true => Vec::new(),
            false => search.hits().iter().take(MAX_HITS).cloned().collect(),
        };

        // The keyboard cursor stays on the result it was on as a new answer reorders the list,
        // and on the same place when that result is gone.
        let picked = self
            .selected
            .and_then(|idx| self.hits.get(idx))
            .map(hit_key);
        self.selected = self.selected.and_then(|idx| {
            let found = picked.and_then(|key| hits.iter().position(|hit| hit_key(hit) == key));
            found.or_else(|| hits.len().checked_sub(1).map(|last| idx.min(last)))
        });
        self.hits = hits;
        if let Some(idx) = self.selected {
            self.scroll.scroll_to_item(idx);
        }
        cx.notify();
    }

    /// Moves the keyboard cursor to the result at `idx` and scrolls it into view. The keyboard
    /// itself stays in the field, so typing goes on refining the query.
    fn choose(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.selected = Some(idx);
        self.scroll.scroll_to_item(idx);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _window: &mut Window, cx: &mut Context<Self>) {
        let total = self.hits.len();
        if total == 0 {
            return;
        }
        let idx = match self.selected {
            None => 0,
            Some(i) => (i + 1).min(total - 1),
        };
        self.choose(idx, cx);
    }

    fn select_previous(
        &mut self,
        _: &SelectPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.selected else {
            return;
        };
        self.choose(current.saturating_sub(1), cx);
    }

    /// Enter: plays the chosen song at once and opens anything else.
    fn activate(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hit) = self.target() {
            enter_hit(&hit, &self.playback, cx);
        }
        self.close(window, cx);
    }

    /// Ctrl+Enter: opens the page of the chosen result, a song's included.
    fn details(&mut self, _: &PowerbarDetails, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hit) = self.target() {
            navigate_hit(&hit, cx);
        }
        self.close(window, cx);
    }

    /// The result the keys act on: the one under the keyboard cursor, or the first.
    fn target(&self) -> Option<Hit> {
        self.selected
            .and_then(|idx| self.hits.get(idx))
            .or_else(|| self.hits.first())
            .cloned()
    }

    /// Whether `hit` is what plays right now, which turns its play button into pause.
    fn playing(&self, hit: &Hit, cx: &App) -> bool {
        match hit {
            Hit::Song(track) => {
                track.id.is_some()
                    && track.id == self.playback_status.0
                    && self.playback_status.1 == Some(true)
            }
            _ => origin_of(hit).is_some_and(|origin| self.playback.read(cx).engaged(&origin, cx)),
        }
    }

    /// The card for one result, marked when the keyboard cursor rests on it and tinted when it
    /// is the song playing now.
    fn hit_row(
        &self,
        hit: &Hit,
        item_idx: usize,
        total: usize,
        on_open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        on_play: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> Card {
        let theme = cx.theme();
        let chosen = self.selected == Some(item_idx);
        let (label, detail, cover, icon) = describe_hit(hit);
        // The list mixes every kind, so each row names its kind ahead of its own detail.
        let subtitle = match detail {
            Some(detail) => SharedString::from(format!("{} {detail}", tag(hit.kind()))),
            None => noun(hit.kind()),
        };
        let current = match hit {
            Hit::Song(track) => track.id.is_some() && track.id == self.playback_status.0,
            _ => false,
        };

        Card::new(("powerbar-hit", item_idx), label)
            .cover(cover)
            .fallback(icon)
            .when(matches!(hit, Hit::Artist(_)), Card::circle)
            .when(current, |card| card.tint(theme.primary))
            .tucked()
            .meta(div().child(subtitle.clone()))
            .aria_description(subtitle)
            .chosen(chosen)
            .aria_position_in_set(item_idx + 1)
            .aria_size_of_set(total)
            .when(chosen, |card| {
                card.aria_selected(true).aria_active_descendant()
            })
            .press(on_open)
            .play(self.playing(hit, cx), on_play)
    }

    /// One row per result in the order the search ranked them, each wrapped in its entrance.
    fn results(&self, window: &Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let total = self.hits.len();
        self.hits
            .iter()
            .enumerate()
            .map(|(item_idx, hit)| {
                let shown = self.cascade.level(&hit_key(hit), window, cx);
                let opened = hit.clone();
                let played = hit.clone();
                let entered = self.playback.clone();
                let playback = self.playback.clone();
                let open_entity = cx.entity();
                let play_entity = cx.entity();
                let card = self.hit_row(
                    hit,
                    item_idx,
                    total,
                    move |_, window, cx| {
                        enter_hit(&opened, &entered, cx);
                        open_entity.update(cx, |this, cx| this.close(window, cx));
                    },
                    move |_, window, cx| {
                        toggle_hit(&played, &playback, cx);
                        play_entity.update(cx, |this, cx| this.close(window, cx));
                    },
                    cx,
                );
                Cascade::surface(card, shown).into_any_element()
            })
            .collect()
    }

    /// What shows under the field while there is a query and nothing to list for it: a spinner
    /// while the answer is on its way, the reason with a retry when the search failed, and "no
    /// matches" once it came back empty.
    fn quiet(&self, cx: &App) -> Option<AnyElement> {
        let search = self.search.read(cx);
        if search.query().is_empty() || !self.hits.is_empty() {
            return None;
        }
        if search.is_loading() {
            return Some(
                div()
                    .flex()
                    .justify_center()
                    .p(cx.theme().metrics.pad * 2.)
                    .child(Spinner::new("powerbar-loading"))
                    .into_any_element(),
            );
        }
        if let Some(reason) = search.error() {
            let again = self.search.clone();
            let query = search.query().to_owned();
            return Some(
                trouble::lost(
                    "powerbar-lost",
                    t!("trouble-not-loaded"),
                    Some(reason),
                    move |_, _, cx| again.update(cx, |search, cx| search.ask(&query, cx)),
                    cx,
                )
                .into_any_element(),
            );
        }
        Some(spoken_vacant("powerbar-empty", t!("search-no-matches"), cx).into_any_element())
    }

    /// The row of key hints along the foot of the panel.
    /// The Enter hint names what Enter does to the result it would act on, and the Ctrl+Enter
    /// hint shows only for a song, since for anything else Enter already opens it.
    fn hints(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let song = matches!(self.target(), Some(Hit::Song(_)));
        let enter = match song {
            true => t!("common-play"),
            false => t!("powerbar-open"),
        };
        let hint = |id: &'static str, keystroke: &'static str, label: SharedString| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .child(Kbd::new(keystroke).spoken(id))
                .child(label)
        };

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_4()
            .gap_y_1()
            .px(theme.metrics.pad * 2.)
            .py(theme.metrics.pad)
            .border_t_1()
            .border_color(theme.border)
            .text_size(theme.text(Text::Small))
            .text_color(theme.muted_foreground)
            .child(hint("powerbar-hint-enter", "enter", enter))
            .when(song, |this| {
                this.child(hint(
                    "powerbar-hint-details",
                    "secondary-enter",
                    t!("powerbar-song"),
                ))
            })
            .child(hint("powerbar-hint-close", "escape", t!("common-dismiss")))
    }
}

impl Render for Powerbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (target, config) = match self.open {
            true => (1., ARRIVE),
            false => (0., LEAVE),
        };
        let shown = self.shown.follow(target, config, REST, window, cx);
        if !self.open && self.shown.resting() {
            return div().into_any_element();
        }

        let theme = *cx.theme();
        let frosted = blurring(cx) && effects();
        let fade = shown.clamp(0., 1.);
        let gone = 1. - fade;

        let chrome = snapped(theme.metrics.title_bar, window);
        let viewport = window.viewport_size();
        let room = viewport.height - chrome;
        let width = (theme.metrics.cover * WIDTH).min(viewport.width - theme.metrics.inset * 2.);
        let radius = corners(theme.radius);
        let reach = room * REACH;

        self.cascade.show(self.hits.iter().map(hit_key));
        let results = self.results(window, cx);
        let quiet = self.quiet(cx);
        let empty = results.is_empty() && quiet.is_none();
        let scroll = self.scroll.clone();

        let body = div()
            .flex()
            .flex_col()
            .w_full()
            .map(|this| match quiet {
                Some(quiet) => this.child(quiet),
                None => this.child(
                    div()
                        .id("powerbar-results")
                        .role(Role::ListBox)
                        .aria_label(t!("search-results"))
                        .flex()
                        .flex_col()
                        .w_full()
                        .max_h(reach)
                        .p(theme.metrics.pad)
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .children(results),
                ),
            })
            // The key hints only make sense with something to act on, so they arrive with the
            // first results and ride the same growing height.
            .when(!self.hits.is_empty(), |this| this.child(self.hints(cx)));
        let body = self.unfold.wrap(body, empty, window, cx);
        let height = self.unfold.height();

        let panel = div()
            .id("powerbar-panel")
            .role(Role::Dialog)
            .aria_label(t!("powerbar-title"))
            .relative()
            .occlude()
            .w_full()
            .flex()
            .flex_col()
            .rounded(radius)
            .border_1()
            .border_color(theme.border)
            .bg(popover_fill(cx))
            .text_color(theme.popover_foreground)
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .px(theme.metrics.pad)
                    .text_size(theme.text(Text::Large))
                    .child(self.input.clone()),
            )
            .child(body.when(height > Pixels::ZERO, |this| {
                this.border_t_1().border_color(theme.border)
            }))
            // The arrival is a paint filter, so the frost cannot ride it and fades in on its own
            // underneath instead.
            .when(shown != 1., |this| {
                this.opacity(fade)
                    .layer_translate(point(px(0.), -DROP * (1. - shown)))
                    .layer_scale(1. - (1. - ZOOM) * (1. - shown))
                    .layer_scale_origin(point(0.5, 0.))
            })
            .map(|this| match BLUR * gone {
                blur if blur > BLUR_LEAST && effects() => this.blur(blur),
                _ => this,
            });

        let scrim = div()
            .id("powerbar-scrim")
            .absolute()
            .inset_0()
            .bg(theme.background.opacity(SCRIM * fade))
            .when(frosted && SCRIM_BLUR * fade > BLUR_LEAST, |this| {
                this.backdrop_blur(SCRIM_BLUR * fade)
            })
            .when(self.open, |this| {
                this.occlude()
                    .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.close(window, cx);
                        }),
                    )
            });

        let column = div()
            .absolute()
            .top(chrome + room * PERCH)
            .left((viewport.width - width) / 2.)
            .w(width)
            .when(frosted, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(radius)
                        .backdrop_blur(FROST)
                        .opacity(fade * fade),
                )
            })
            .child(panel);

        let frame = div()
            .relative()
            .w(viewport.width)
            .h(viewport.height)
            .when(self.open, |this| {
                this.key_context(POWERBAR_CONTEXT)
                    .on_action(cx.listener(Self::select_next))
                    .on_action(cx.listener(Self::select_previous))
                    .on_action(cx.listener(Self::activate))
                    .on_action(cx.listener(Self::details))
                    .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                        cx.stop_propagation();
                        this.close(window, cx);
                    }))
            })
            .child(scrim)
            .child(column);

        deferred(
            anchored()
                .position(point(Pixels::ZERO, Pixels::ZERO))
                .child(frame),
        )
        .into_any_element()
    }
}

/// What tells one result from another across answers, so a result that stays keeps its place
/// in the entrance.
fn hit_key(hit: &Hit) -> SharedString {
    let (kind, id) = match hit {
        Hit::Song(track) => ("song", track.id.as_deref().unwrap_or(&track.name)),
        Hit::Artist(artist) => ("artist", artist.id.as_deref().unwrap_or(&artist.name)),
        Hit::Album(album) => ("album", album.id.as_str()),
        Hit::Playlist(list) => ("playlist", list.id.as_str()),
    };
    SharedString::from(format!("{kind}:{id}"))
}

fn navigate_hit(hit: &Hit, cx: &mut App) {
    match hit {
        Hit::Song(track) => {
            if let Some(id) = &track.id {
                navigate(Destination::Song(SharedString::from(id.clone())), cx);
            }
        }
        Hit::Artist(artist) => {
            if let Some(id) = &artist.id {
                navigate(Destination::Artist(SharedString::from(id.clone())), cx);
            }
        }
        Hit::Album(album) => {
            navigate(Destination::Album(SharedString::from(album.id.clone())), cx);
        }
        Hit::Playlist(list) => {
            navigate(
                Destination::Playlist(SharedString::from(list.id.clone())),
                cx,
            );
        }
    }
}

/// The origin that playing `hit` starts, for every kind but a song, which starts a radio.
fn origin_of(hit: &Hit) -> Option<Origin> {
    match hit {
        Hit::Song(_) => None,
        Hit::Artist(artist) => Some(Origin::artist(artist.id.clone()?).named(artist.name.clone())),
        Hit::Album(album) => Some(Origin::album(album.id.clone()).named(album.name.clone())),
        Hit::Playlist(list) => Some(Origin::playlist(list.id.clone()).named(list.name.clone())),
    }
}

/// What Enter or a click on a row does: a song starts playing with its radio after it, and
/// any other result opens its page.
fn enter_hit(hit: &Hit, playback: &Entity<Playback>, cx: &mut App) {
    match hit {
        Hit::Song(track) => playback.update(cx, |playback, cx| playback.play_radio(track, cx)),
        _ => navigate_hit(hit, cx),
    }
}

/// The play button on a result: pauses or resumes what already plays from it, and starts it
/// otherwise.
fn toggle_hit(hit: &Hit, playback: &Entity<Playback>, cx: &mut App) {
    playback.update(cx, |playback, cx| match (hit, origin_of(hit)) {
        (Hit::Song(track), _) => {
            let current = track.id.is_some()
                && playback.track().and_then(|playing| playing.id.as_ref()) == track.id.as_ref();
            match current {
                true => playback.toggle_play(cx),
                false => playback.play_radio(track, cx),
            }
        }
        (_, Some(origin)) => playback.toggle_origin(&origin, cx),
        (_, None) => {}
    });
}

/// The title, the detail line, the cover and the fallback icon of a result. The detail is
/// `None` where the kind alone says enough, as for an artist or a playlist with no owner.
fn describe_hit(
    hit: &Hit,
) -> (
    SharedString,
    Option<SharedString>,
    Option<String>,
    &'static str,
) {
    match hit {
        Hit::Song(track) => (
            SharedString::from(track.name.clone()),
            Some(SharedString::from(format!(
                "{} · {}",
                track.artists, track.album
            ))),
            track.cover.clone(),
            "icons/music.svg",
        ),
        Hit::Artist(artist) => (
            SharedString::from(artist.name.clone()),
            None,
            artist.cover.clone(),
            "icons/user.svg",
        ),
        Hit::Album(album) => (
            SharedString::from(album.name.clone()),
            Some(SharedString::from(album.artists.clone())),
            album.cover.clone(),
            "icons/disc-3.svg",
        ),
        Hit::Playlist(list) => (
            SharedString::from(list.name.clone()),
            (!list.owner.is_empty()).then(|| SharedString::from(list.owner.clone())),
            list.cover.clone(),
            "icons/list.svg",
        ),
    }
}
