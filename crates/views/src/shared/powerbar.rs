use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, Entity, FocusHandle, Global, MouseButton, Render, ScrollHandle,
    SharedString, Window, div, px, svg,
};
use i18n::t;
use input::{POWERBAR_CONTEXT, PowerbarConfirm, PowerbarNextCategory, PowerbarPrevCategory};
use router::{Destination, navigate};
use state::{Hit, Kind, Origin, Playback, Search};
use ui::{
    ActiveTheme as _, Artwork, Avatar, Dismiss, Input, Modal, SelectNext, SelectPrevious, Submit,
    Text, eyebrow,
};

use crate::shared::tracks::{PlaybackStatus, playback_status};

/// Maximum results shown in the powerbar per category.
const MAX_PER_KIND: usize = 3;

pub(crate) struct Powerbar {
    open: bool,
    input: Entity<Input>,
    search: Entity<Search>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    grouped: Vec<(Kind, Vec<Hit>)>,
    selected: Option<usize>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    scroll: ScrollHandle,
    item_to_child: Vec<usize>,
    category_starts: Vec<usize>,
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
                        .icon("icons/search.svg")
                        .clearable()
                });
                cx.observe(&input, |this: &mut Powerbar, input, cx| {
                    let query = input.read(cx).text().to_owned();
                    this.search.update(cx, |search, cx| search.ask(&query, cx));
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
                    grouped: Vec::new(),
                    selected: None,
                    focus: cx.focus_handle(),
                    restore: None,
                    scroll: ScrollHandle::new(),
                    item_to_child: Vec::new(),
                    category_starts: Vec::new(),
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

    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore = window.focused(cx);
        self.open = true;
        self.selected = None;
        self.input.update(cx, |input, cx| input.focus(window, cx));
        let query = self.input.read(cx).text().to_owned();
        self.search.update(cx, |search, cx| search.ask(&query, cx));
        cx.notify();
    }

    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.selected = None;
        self.input.update(cx, |input, cx| input.set_text("", cx));
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn rebuild_items(&mut self, cx: &mut Context<Self>) {
        let search = self.search.read(cx);
        if search.query().trim().is_empty() {
            self.grouped.clear();
            self.item_to_child.clear();
            self.category_starts.clear();
            self.selected = None;
            cx.notify();
            return;
        }

        let mut grouped = Vec::with_capacity(Kind::ALL.len());
        let mut item_to_child = Vec::new();
        let mut category_starts = Vec::new();
        let mut child_count = 0;
        let mut flat_idx = 0;

        for kind in Kind::ALL {
            let hits: Vec<Hit> = search.of(kind).take(MAX_PER_KIND).cloned().collect();
            if hits.is_empty() {
                continue;
            }
            category_starts.push(flat_idx);
            child_count += 1;
            for _ in &hits {
                item_to_child.push(child_count);
                child_count += 1;
                flat_idx += 1;
            }
            grouped.push((kind, hits));
        }

        self.grouped = grouped;
        self.item_to_child = item_to_child;
        self.category_starts = category_starts;
        self.selected = None;
        cx.notify();
    }

    fn total_items(&self) -> usize {
        self.item_to_child.len()
    }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        let total = self.total_items();
        if total == 0 {
            return;
        }
        let idx = match self.selected {
            None => 0,
            Some(i) => (i + 1).min(total - 1),
        };
        self.selected = Some(idx);
        if let Some(&child) = self.item_to_child.get(idx) {
            self.scroll.scroll_to_item(child);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn select_previous(&mut self, _: &SelectPrevious, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected {
            None => {}
            Some(0) => {
                self.selected = None;
                self.scroll.scroll_to_item(0);
                self.input.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }
            Some(i) => {
                let idx = i - 1;
                self.selected = Some(idx);
                if let Some(&child) = self.item_to_child.get(idx) {
                    self.scroll.scroll_to_item(child);
                }
                window.focus(&self.focus, cx);
                cx.notify();
            }
        }
    }

    fn select_next_category(
        &mut self,
        _: &PowerbarNextCategory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.category_starts.is_empty() {
            return;
        }
        let current = self.selected.unwrap_or(0);
        let next = self
            .category_starts
            .iter()
            .copied()
            .find(|&idx| idx > current)
            .unwrap_or(self.category_starts[0]);
        self.selected = Some(next);
        if let Some(&child) = self.item_to_child.get(next) {
            self.scroll.scroll_to_item(child);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn select_prev_category(
        &mut self,
        _: &PowerbarPrevCategory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.category_starts.is_empty() {
            return;
        }
        let Some(&last) = self.category_starts.last() else {
            return;
        };
        let current = self.selected.unwrap_or(0);
        let prev = self
            .category_starts
            .iter()
            .copied()
            .rfind(|&idx| idx < current)
            .unwrap_or(last);
        self.selected = Some(prev);
        if let Some(&child) = self.item_to_child.get(prev) {
            self.scroll.scroll_to_item(child);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn activate(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hit) = self.selected_hit().or_else(|| self.first_hit()) {
            navigate_hit(&hit, cx);
        }
        self.close(window, cx);
    }

    fn play_confirm(&mut self, _: &PowerbarConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(hit) = self.selected_hit().or_else(|| self.first_hit()) {
            play_hit(&hit, &self.playback, cx);
        }
        self.close(window, cx);
    }

    fn selected_hit(&self) -> Option<Hit> {
        let idx = self.selected?;
        self.hit_at(idx)
    }

    fn first_hit(&self) -> Option<Hit> {
        self.hit_at(0)
    }

    fn hit_at(&self, mut idx: usize) -> Option<Hit> {
        for (_, hits) in &self.grouped {
            if idx < hits.len() {
                return hits.get(idx).cloned();
            }
            idx -= hits.len();
        }
        None
    }
}

fn category_title(kind: Kind) -> SharedString {
    match kind {
        Kind::Song => t!("search-songs"),
        Kind::Artist => t!("search-artists"),
        Kind::Album => t!("nav-albums"),
        Kind::Playlist => t!("nav-playlists"),
    }
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

fn play_hit(hit: &Hit, playback: &Entity<Playback>, cx: &mut App) {
    playback.update(cx, |playback, cx| match hit {
        Hit::Song(track) => playback.play_radio(track, cx),
        Hit::Artist(artist) => {
            if let Some(id) = &artist.id {
                playback.play_origin(Origin::artist(id.clone()).named(artist.name.clone()), cx);
            }
        }
        Hit::Album(album) => {
            playback.play_origin(
                Origin::album(album.id.clone()).named(album.name.clone()),
                cx,
            );
        }
        Hit::Playlist(list) => {
            playback.play_origin(
                Origin::playlist(list.id.clone()).named(list.name.clone()),
                cx,
            );
        }
    });
}

impl Render for Powerbar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }

        let theme = *cx.theme();
        let selected = self.selected;

        let mut flat_children: Vec<AnyElement> = Vec::new();
        let mut flat_idx = 0;

        for (g_idx, (kind, group_items)) in self.grouped.iter().enumerate() {
            flat_children.push(
                div()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .when(g_idx > 0, |d| d.border_t_1().border_color(theme.border))
                    .child(eyebrow(category_title(*kind), cx))
                    .into_any_element(),
            );
            for hit in group_items {
                let item_idx = flat_idx;
                let is_chosen = selected == Some(item_idx);
                let hit_clone = hit.clone();
                let hit_play = hit.clone();
                let playback = self.playback.clone();
                let this_entity = cx.entity().clone();
                flat_children.push(
                    hit_row(hit, item_idx, is_chosen, &theme, move |window, cx| {
                        play_hit(&hit_play, &playback, cx);
                        this_entity.update(cx, |this, cx| this.close(window, cx));
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            navigate_hit(&hit_clone, cx);
                            this.close(window, cx);
                        }),
                    )
                    .into_any_element(),
                );
                flat_idx += 1;
            }
        }

        let scroll = self.scroll.clone();

        div()
            .absolute()
            .inset_0()
            .key_context(POWERBAR_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_next_category))
            .on_action(cx.listener(Self::select_prev_category))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::play_confirm))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .child(
                Modal::new("powerbar", t!("powerbar-title"))
                    .w(theme.metrics.cover * 5.0)
                    .child(self.input.clone())
                    .when(!flat_children.is_empty(), |modal| {
                        modal.child(
                            div()
                                .id("powerbar-results")
                                .flex()
                                .flex_col()
                                .w_full()
                                .overflow_y_scroll()
                                .track_scroll(&scroll)
                                .children(flat_children),
                        )
                    })
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}

fn hit_row(
    hit: &Hit,
    item_idx: usize,
    chosen: bool,
    theme: &ui::Theme,
    on_play: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let (label, subtitle, cover, icon) = describe_hit(hit);

    let bg = match chosen {
        true => theme.secondary,
        false => theme.background,
    };

    let thumb_size = theme.metrics.thumb;
    let is_artist = matches!(hit, Hit::Artist(_));
    let visual = match is_artist {
        true => Avatar::new(cover).size(thumb_size).into_any_element(),
        false => Artwork::new(cover)
            .size(thumb_size)
            .fallback(icon)
            .into_any_element(),
    };

    let corner = match is_artist {
        true => thumb_size / 2.,
        false => theme.radius,
    };

    let row_group: SharedString = format!("row-{item_idx}").into();

    let thumbnail = div()
        .relative()
        .flex_none()
        .size(thumb_size)
        .child(visual)
        .child(
            div()
                .id(("hit-play-scrim", item_idx))
                .absolute()
                .inset_0()
                .rounded(corner)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .invisible()
                .hover(|style| style.visible().bg(theme.overlay))
                .group_hover(row_group.clone(), |style| style.visible().bg(theme.overlay))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    on_play(window, cx);
                })
                .child(
                    svg()
                        .path(icons::path("icons/play-filled.svg"))
                        .size(px(16.))
                        .text_color(theme.overlay_foreground),
                ),
        );

    div()
        .group(row_group)
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .cursor_pointer()
        .bg(bg)
        .hover(|style| match chosen {
            true => style,
            false => style.bg(theme.secondary_hover),
        })
        .child(thumbnail)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(theme.text(Text::Body))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(label),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(theme.text(Text::Small))
                        .text_color(theme.muted_foreground)
                        .child(subtitle),
                ),
        )
}

fn describe_hit(hit: &Hit) -> (SharedString, SharedString, Option<String>, &'static str) {
    match hit {
        Hit::Song(track) => (
            SharedString::from(track.name.clone()),
            SharedString::from(format!("{} · {}", track.artists, track.album)),
            track.cover.clone(),
            "icons/music.svg",
        ),
        Hit::Artist(artist) => (
            SharedString::from(artist.name.clone()),
            t!("artist-eyebrow"),
            artist.cover.clone(),
            "icons/user.svg",
        ),
        Hit::Album(album) => (
            SharedString::from(album.name.clone()),
            SharedString::from(album.artists.clone()),
            album.cover.clone(),
            "icons/disc-3.svg",
        ),
        Hit::Playlist(list) => (
            SharedString::from(list.name.clone()),
            SharedString::from(list.owner.clone()),
            list.cover.clone(),
            "icons/list.svg",
        ),
    }
}
