//! The dialog for matching lyrics by hand (sonorahq/sonora#1127): search the enabled sources
//! with an edited title and artist, preview what they answer, and keep the right sheet for the
//! song. The choice is bound to the track by the state and outlives the automatic answers.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Global, Render, SharedString, Window, div,
};
use i18n::t;
use music::{LyricsHit, Track};
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
    track: Option<Track>,
    last_title: String,
    last_artist: String,
}

struct Installed(Entity<LyricsSearch>);

impl Global for Installed {}

impl LyricsSearch {
    pub(crate) fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let dialog = cx.new(|cx| {
                let lyrics = Sonora::global(cx).lyrics.clone();
                let playback = Sonora::global(cx).playback.clone();
                let title = cx.new(|cx| Input::new("lyrics-search-hint-title", cx));
                let artist = cx.new(|cx| Input::new("lyrics-search-hint-artist", cx));

                cx.observe(&lyrics, |_, _, cx| cx.notify()).detach();

                cx.observe(&playback, |this: &mut LyricsSearch, playback, cx| {
                    let shown = playback.read(cx).shown_track(cx);
                    if should_reset_on_track_change(this.open, shown.as_ref(), this.track.as_ref())
                    {
                        this.reset_for_track(shown, cx);
                        cx.notify();
                    }
                })
                .detach();

                cx.observe(&title, |this: &mut LyricsSearch, input, cx| {
                    let text = input.read(cx).text();
                    if should_invalidate_on_edit(this.open, text, &this.last_title) {
                        this.last_title = text.to_owned();
                        this.previewed = None;
                        this.lyrics
                            .update(cx, |lyrics, cx| lyrics.cancel_search(cx));
                        cx.notify();
                    }
                })
                .detach();

                cx.observe(&artist, |this: &mut LyricsSearch, input, cx| {
                    let text = input.read(cx).text();
                    if should_invalidate_on_edit(this.open, text, &this.last_artist) {
                        this.last_artist = text.to_owned();
                        this.previewed = None;
                        this.lyrics
                            .update(cx, |lyrics, cx| lyrics.cancel_search(cx));
                        cx.notify();
                    }
                })
                .detach();

                Self {
                    open: false,
                    lyrics,
                    title,
                    artist,
                    previewed: None,
                    row_focus: Vec::new(),
                    focus: cx.focus_handle(),
                    restore: None,
                    track: None,
                    last_title: String::new(),
                    last_artist: String::new(),
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
            this.open = true;
            this.reset_for_track(Some(track), cx);
            this.title.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        });
    }

    fn reset_for_track(&mut self, track: Option<Track>, cx: &mut Context<Self>) {
        self.track = track.clone();
        self.previewed = None;
        self.lyrics
            .update(cx, |lyrics, cx| lyrics.cancel_search(cx));
        if let Some(track) = track {
            self.last_title = track.name.clone();
            self.last_artist = track.artists.clone();
            self.title
                .update(cx, |input, cx| input.set_text(track.name, cx));
            self.artist
                .update(cx, |input, cx| input.set_text(track.artists, cx));
        } else {
            self.last_title.clear();
            self.last_artist.clear();
            self.title.update(cx, |input, cx| input.set_text("", cx));
            self.artist.update(cx, |input, cx| input.set_text("", cx));
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.track = None;
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
                SearchState::Unanswered => status("lyrics-search-unanswered", cx),
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
        if self.previewed.is_some() && matches!(state, SearchState::Ready) {
            modal = modal.action(
                Button::new("back-to-lyrics-results")
                    .outline()
                    .label(t!("lyrics-search-back"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.previewed = None;
                        cx.notify();
                    })),
            );
        }
        modal = match self.previewed.clone() {
            Some(hit) if can_bind(Some(&hit), &state) => modal.action(
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
                let state = this.lyrics.read(cx).search_state().clone();
                match this.previewed.clone() {
                    Some(hit) if can_bind(Some(&hit), &state) => this.bind(hit, window, cx),
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

/// Whether a search hit can be bound to the track. A hit can only be bound when one is
/// currently on preview and the search results are ready.
fn can_bind(previewed: Option<&LyricsHit>, state: &SearchState) -> bool {
    previewed.is_some() && matches!(state, SearchState::Ready)
}

/// Whether a playback update requires the dialog to reset its fields and clear results.
/// The dialog resets only while open and when the shown track has changed from what the dialog holds.
fn should_reset_on_track_change(
    open: bool,
    shown: Option<&Track>,
    current: Option<&Track>,
) -> bool {
    open && shown != current
}

/// Whether an input notification represents an actual text change that invalidates current
/// search results. Caret moves or selections where the text is unchanged do not invalidate.
fn should_invalidate_on_edit(open: bool, current: &str, last: &str) -> bool {
    open && current != last
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_track(name: &str, artists: &str) -> Track {
        Track {
            id: Some(format!("test:{name}")),
            name: name.to_owned(),
            playable: true,
            artists: artists.to_owned(),
            album: "Test Album".to_owned(),
            duration: Duration::from_secs(180),
            ..Default::default()
        }
    }

    fn test_hit(title: &str, artist: &str) -> LyricsHit {
        LyricsHit {
            source: "Test",
            title: title.to_owned(),
            artist: artist.to_owned(),
            duration: Some(Duration::from_secs(180)),
            lyrics: music::Lyrics::Plain {
                text: "test lyrics".to_owned(),
                romanized: None,
            },
            ..Default::default()
        }
    }

    #[test]
    fn track_change_helper_decides_reset_for_open_dialog_and_ignores_closed_or_identical() {
        let track_a = test_track("Song A", "Artist A");
        let track_b = test_track("Song B", "Artist B");

        // When dialog is closed (open = false), a track change must never trigger a reset:
        assert!(!should_reset_on_track_change(
            false,
            Some(&track_b),
            Some(&track_a)
        ));
        assert!(!should_reset_on_track_change(false, Some(&track_a), None));
        assert!(!should_reset_on_track_change(false, None, Some(&track_a)));
        assert!(!should_reset_on_track_change(
            false,
            Some(&track_a),
            Some(&track_a)
        ));
        assert!(!should_reset_on_track_change(false, None, None));

        // When dialog is open (open = true):
        // 1. Changing to a different track must trigger a reset:
        assert!(should_reset_on_track_change(
            true,
            Some(&track_b),
            Some(&track_a)
        ));

        // 2. A track appearing when the dialog held none must trigger a reset:
        assert!(should_reset_on_track_change(true, Some(&track_a), None));

        // 3. Playback stopping (track becoming None) while dialog holds a track must trigger a reset:
        assert!(should_reset_on_track_change(true, None, Some(&track_a)));

        // 4. Same track (position ticks, volume changes, pauses) must NOT trigger a reset:
        assert!(!should_reset_on_track_change(
            true,
            Some(&track_a),
            Some(&track_a)
        ));

        // 5. Both None (playback idle, update occurs) must NOT trigger a reset:
        assert!(!should_reset_on_track_change(true, None, None));
    }

    #[test]
    fn edit_helper_decides_invalidation_only_on_text_changes_while_open() {
        // When dialog is closed (open = false), edits must never trigger invalidation:
        assert!(!should_invalidate_on_edit(false, "New Title", "Old Title"));
        assert!(!should_invalidate_on_edit(
            false,
            "Same Title",
            "Same Title"
        ));
        assert!(!should_invalidate_on_edit(false, "", "Old Title"));

        // When dialog is open (open = true):
        // 1. Text edits (title or artist) must trigger invalidation:
        assert!(should_invalidate_on_edit(true, "New Title", "Old Title"));
        assert!(should_invalidate_on_edit(true, "New Artist", "Old Artist"));
        assert!(should_invalidate_on_edit(true, "", "Old Title"));
        assert!(should_invalidate_on_edit(true, "A", ""));

        // 2. Non-edit notifications (caret movement, selection, focus) where text is unchanged must NOT invalidate:
        assert!(!should_invalidate_on_edit(true, "Old Title", "Old Title"));
        assert!(!should_invalidate_on_edit(true, "Old Artist", "Old Artist"));
        assert!(!should_invalidate_on_edit(true, "", ""));
    }

    #[test]
    fn same_hit_matches_identical_results_across_reranking() {
        let hit1 = test_hit("Song", "Artist");
        let hit2 = test_hit("Song", "Artist");
        let mut hit3 = test_hit("Song", "Other Artist");
        hit3.source = "OtherSource";

        assert!(same_hit(&hit1, &hit2));
        assert!(!same_hit(&hit1, &hit3));
    }
}
