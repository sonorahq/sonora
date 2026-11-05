//! The dialog for matching lyrics by hand (sonorahq/sonora#1127): search the enabled sources
//! with an edited title and artist, preview what they answer, and keep the right sheet for the
//! song. The choice is bound to the track by the state and outlives the automatic answers.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Global, Render, SharedString, Window, div,
};
use i18n::t;
use music::LyricsHit;
use state::{Lyrics, SearchState, Sonora};
use ui::{ActiveTheme as _, Button, Dismiss, FORM_CONTEXT, Input, Modal, Submit, Text};

use crate::shared::quiet;

pub(crate) struct LyricsSearch {
    open: bool,
    lyrics: Entity<Lyrics>,
    title: Entity<Input>,
    artist: Entity<Input>,
    /// The result whose sheet is on preview, ready to bind. The hit itself is kept, not its
    /// position: the results keep reranking as late answers come in, and an index into the
    /// live list could come to mean a different sheet than the one on preview.
    previewed: Option<LyricsHit>,
    /// A focus handle per result row, so a row can take the focus and answer Enter.
    row_focus: Vec<FocusHandle>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
}

struct Installed(Entity<LyricsSearch>);

impl Global for Installed {}

impl LyricsSearch {
    pub(crate) fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let dialog = cx.new(|cx| {
                let lyrics = Sonora::global(cx).lyrics.clone();
                cx.observe(&lyrics, |_, _, cx| cx.notify()).detach();
                Self {
                    open: false,
                    lyrics,
                    title: cx.new(|cx| Input::new("lyrics-search-hint-title", cx)),
                    artist: cx.new(|cx| Input::new("lyrics-search-hint-artist", cx)),
                    previewed: None,
                    row_focus: Vec::new(),
                    focus: cx.focus_handle(),
                    restore: None,
                }
            });
            cx.set_global(Installed(dialog));
        }
        cx.global::<Installed>().0.clone()
    }

    pub(crate) fn open(window: &mut Window, cx: &mut App) {
        let dialog = Self::entity(cx);
        dialog.update(cx, |this, cx| {
            let Some(track) = Sonora::global(cx).playback.read(cx).shown_track(cx) else {
                return;
            };
            this.restore = window.focused(cx);
            this.previewed = None;
            this.title
                .update(cx, |input, cx| input.set_text(track.name, cx));
            this.artist
                .update(cx, |input, cx| input.set_text(track.artists, cx));
            this.open = true;
            this.title.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        });
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.previewed = None;
        self.lyrics
            .update(cx, |lyrics, cx| lyrics.cancel_search(cx));
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let title = self.title.read(cx).text().to_owned();
        let artist = self.artist.read(cx).text().to_owned();
        self.previewed = None;
        self.lyrics
            .update(cx, |lyrics, cx| lyrics.search_manually(title, artist, cx));
        cx.notify();
    }

    fn bind(&mut self, hit: LyricsHit, window: &mut Window, cx: &mut Context<Self>) {
        self.lyrics.update(cx, |lyrics, cx| lyrics.bind(hit, cx));
        self.close(window, cx);
    }

    fn unbind(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lyrics.update(cx, |lyrics, cx| lyrics.unbind(cx));
        self.close(window, cx);
    }

    /// The rows the search answered with, each naming its source and what it thinks it is. A
    /// row is a real button: it takes the focus, and Enter on it previews the sheet, then
    /// binds it, the way clicking the row and then Use does. What a click or a keypress acts
    /// on is the hit the row was drawn with, never its index, so an answer arriving late and
    /// reranking the list cannot turn a pick into a different sheet.
    fn results(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let hits = self.lyrics.read(cx).search_hits().to_vec();
        let total = hits.len();
        while self.row_focus.len() < total {
            self.row_focus.push(cx.focus_handle().tab_stop(true));
        }
        self.row_focus.truncate(total);
        div()
            .id("lyrics-search-results")
            .role(Role::List)
            .aria_label(t!("lyrics-search-results"))
            .flex()
            .flex_col()
            .w_full()
            .gap_1()
            .children(hits.into_iter().enumerate().map(|(index, hit)| {
                let kind = match hit.lyrics.synced() {
                    true => t!("lyrics-search-synced"),
                    false => t!("lyrics-search-plain"),
                };
                let row_focus = self.row_focus[index].clone();
                let clicked = hit.clone();
                let pressed = hit.clone();
                let chosen = self
                    .previewed
                    .as_ref()
                    .is_some_and(|previewed| same_hit(previewed, &hit));
                div()
                    .id(("lyrics-search-hit", index))
                    .role(Role::Button)
                    .track_focus(&row_focus)
                    .aria_label(format!("{}, {}, {kind}", hit.title, hit.artist,))
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(total)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .w_full()
                    .rounded(theme.radius)
                    .p_2()
                    .bg(theme.background)
                    .hover(|this| this.bg(theme.secondary_hover))
                    .when(chosen, |this| this.bg(theme.secondary_active))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_ellipsis()
                                    .child(SharedString::from(hit.title.clone())),
                            )
                            .child(
                                div()
                                    .text_size(theme.text(Text::Small))
                                    .text_color(quiet(&theme))
                                    .text_ellipsis()
                                    .child(SharedString::from(hit.artist.clone())),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(theme.text(Text::Small))
                            .text_color(quiet(&theme))
                            .child(SharedString::from(hit.source))
                            .when_some(hit.duration, |this, duration| {
                                this.child(ui::runtime(duration))
                            })
                            .child(kind),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&row_focus, cx);
                        this.previewed = Some(clicked.clone());
                        cx.notify();
                    }))
                    .on_action(cx.listener(move |this, _: &Submit, window, cx| {
                        cx.stop_propagation();
                        let hit = pressed.clone();
                        let chosen = this
                            .previewed
                            .as_ref()
                            .is_some_and(|previewed| same_hit(previewed, &hit));
                        match chosen {
                            true => this.bind(hit, window, cx),
                            false => {
                                this.previewed = Some(hit);
                                cx.notify();
                            }
                        }
                    }))
            }))
            .into_any_element()
    }

    /// The sheet of the result on preview, as plain reading text.
    fn preview(&self, hit: &LyricsHit, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let text = match &hit.lyrics {
            music::Lyrics::Plain { text, .. } => text.clone(),
            music::Lyrics::Synced { lines } => lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        };
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .child(
                div()
                    .id("lyrics-search-preview")
                    .role(Role::Paragraph)
                    .aria_label(SharedString::from(text.clone()))
                    .w_full()
                    .rounded(theme.radius)
                    .p_2()
                    .bg(theme.background)
                    .text_size(theme.text(Text::Small))
                    .child(SharedString::from(text)),
            )
            .child(
                div()
                    .text_size(theme.text(Text::Small))
                    .text_color(quiet(&theme))
                    .child(t!(
                        "lyrics-search-preview-of",
                        source = hit.source,
                        title = &hit.title,
                        artist = &hit.artist
                    )),
            )
            .into_any_element()
    }
}

impl Render for LyricsSearch {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let theme = *cx.theme();
        let state = self.lyrics.read(cx).search_state().clone();
        let searching = matches!(state, SearchState::Searching);
        let matched = self.lyrics.read(cx).manual_match();
        let can_search = !searching
            && !matches!(state, SearchState::Offline)
            && !self.title.read(cx).text().trim().is_empty();

        let mut modal = Modal::new("lyrics-search", t!("lyrics-search-title"))
            .w(theme.metrics.cover * 2.8)
            .close_button();
        if let Some((_, _, hit)) = &matched {
            modal = modal.detail(t!("lyrics-search-current", source = hit.source));
        }
        modal = modal
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap_2()
                    .child(self.title.clone())
                    .child(self.artist.clone()),
            )
            .child(match &state {
                SearchState::Searching => status("lyrics-search-searching", cx),
                SearchState::Ready if self.lyrics.read(cx).search_hits().is_empty() => {
                    status("lyrics-search-none", cx)
                }
                SearchState::Ready => match self.previewed.clone() {
                    Some(hit) => self.preview(&hit, cx),
                    None => self.results(cx),
                },
                SearchState::Unsupported => status("lyrics-search-unsupported", cx),
                SearchState::Offline => status("lyrics-search-offline", cx),
                SearchState::Failed(reason) => status_text(reason.clone().into(), cx),
                SearchState::Idle => status("lyrics-search-start", cx),
            });

        modal = modal.action(
            Button::new("cancel-lyrics-search")
                .ghost()
                .label(t!("common-cancel"))
                .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
        );
        if matched.is_some() {
            modal = modal.action(
                Button::new("reset-lyrics-match")
                    .outline()
                    .icon("icons/undo-2.svg")
                    .label(t!("lyrics-search-reset"))
                    .on_click(cx.listener(|this, _, window, cx| this.unbind(window, cx))),
            );
        }
        modal = match self.previewed.clone() {
            Some(hit) if matches!(state, SearchState::Ready) => modal.action(
                Button::new("use-lyrics-match")
                    .primary()
                    .label(t!("lyrics-search-use"))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.bind(hit.clone(), window, cx)),
                    ),
            ),
            _ => modal.action(
                Button::new("run-lyrics-search")
                    .primary()
                    .icon("icons/search.svg")
                    .label(t!("lyrics-search-go"))
                    .disabled(!can_search)
                    .on_click(cx.listener(|this, _, _, cx| this.search(cx))),
            ),
        };

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                cx.stop_propagation();
                match this.previewed.clone() {
                    Some(hit)
                        if matches!(this.lyrics.read(cx).search_state(), SearchState::Ready) =>
                    {
                        this.bind(hit, window, cx)
                    }
                    _ => this.search(cx),
                }
            }))
            .child(modal.on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))))
            .into_any_element()
    }
}

/// A one-line status for the dialog's empty states, named for screen readers.
fn status(key: &'static str, cx: &mut Context<LyricsSearch>) -> AnyElement {
    status_text(i18n::lookup(key, None), cx)
}

/// Whether two search answers are the same match. The preview is kept beside a list that can
/// rerank as sources answer, so it is found again by what it is, not where it was.
fn same_hit(left: &LyricsHit, right: &LyricsHit) -> bool {
    left.source == right.source
        && left.title == right.title
        && left.artist == right.artist
        && left.album == right.album
        && left.duration == right.duration
        && left.lyrics.worded() == right.lyrics.worded()
}

fn status_text(text: SharedString, cx: &mut Context<LyricsSearch>) -> AnyElement {
    let theme = *cx.theme();
    div()
        .id("lyrics-search-status")
        .role(Role::Label)
        .aria_value(text.clone())
        .w_full()
        .py_4()
        .text_center()
        .text_color(quiet(&theme))
        .child(text)
        .into_any_element()
}
