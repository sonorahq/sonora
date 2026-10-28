use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{App, Div, ElementId, Entity, MouseButton, StyleRefinement, Window, div, svg};
use i18n::t;
use music::Track;
use state::{Playback, Sonora};
use ui::{ActiveTheme as _, Button, Perch, Tipped as _};

/// The star's size as a fraction of its hit area's height.
const MARK: f32 = 0.62;

/// A track's heart and five stars. All ratings and hover values are integers from 1 to 10.
#[derive(IntoElement)]
pub(crate) struct Rating {
    base: Div,
    id: ElementId,
    track: Track,
    playback: Option<Entity<Playback>>,
    frosted: bool,
}

/// Shared by track rows, menus and players. Player clicks act on the track playing at click time.
pub(crate) fn rating(
    id: impl Into<ElementId>,
    track: Option<&Track>,
    playback: Option<&Entity<Playback>>,
    frosted: bool,
) -> Option<Rating> {
    track.map(|track| Rating {
        base: div(),
        id: id.into(),
        track: track.clone(),
        playback: playback.cloned(),
        frosted,
    })
}

impl Styled for Rating {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Rating {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let sonora = Sonora::global(cx);
        let entity = sonora.ratings.clone();
        let ratings = entity.read(cx);
        let track_id = self.track.id.as_deref().unwrap_or_default();
        if track_id.is_empty() || music::is_local_id(track_id) || !ratings.enabled(cx) {
            return self.like(None, cx).into_any_element();
        }
        if ratings.failed() {
            return Button::new(self.id)
                .ghost()
                .small()
                .icon("icons/refresh-cw.svg")
                .tooltip_above("ratings-retry")
                .on_click(move |_, _, cx| entity.update(cx, |ratings, cx| ratings.retry(cx)))
                .into_any_element();
        }
        let busy = ratings.loading() || ratings.pending(track_id);
        let value = ratings.for_track(track_id);
        let options = sonora.settings.read(cx).star_ratings();
        let hovered = window.use_keyed_state((self.id.clone(), "hover"), cx, |_, _| None::<u8>);
        let hovering = (*hovered.read(cx)).filter(|_| !busy);
        let preview = hovering.or(value).unwrap_or(0);
        let saved = hovering
            .zip(options.auto_like)
            .map(|(value, at)| value >= at);
        let heart = self.like(saved, cx);
        let theme = *cx.theme();
        let hit = theme.metrics.control_small;
        let half = ui::snapped(hit * MARK / 2., window);
        let side = half * 2.;
        let top = ui::snapped((hit - side) / 2., window);
        let gap = theme.metrics.pad / 4.;
        let ink = hovering.map_or(theme.foreground, |_| theme.primary);
        let reset = hovered.clone();
        let overrides = std::mem::take(self.base.style());
        let mut row = self
            .base
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .child(div().mr(gap).child(heart))
            .on_hover(move |over, _, cx| {
                if !over {
                    reset.write(cx, None);
                }
            })
            .when(busy, |row| {
                row.opacity(0.5).tip("ratings-loading", Perch::Above)
            })
            .children((1..=10u8).map(|rating| {
                let selection = match options.half_ratings {
                    true => rating,
                    false => rating.next_multiple_of(2),
                };
                let label = match value == Some(selection) {
                    true => t!("ratings-clear"),
                    false => t!("ratings-set", rating = f64::from(selection) / 2.),
                };
                let hover = hovered.clone();
                let entity = entity.clone();
                let playback = self.playback.clone();
                let track = self.track.clone();
                div()
                    .id(("rating", usize::from(rating)))
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .relative()
                    .flex_none()
                    .w(half)
                    .h(hit)
                    .when(rating > 1 && !rating.is_multiple_of(2), |half| half.ml(gap))
                    .overflow_hidden()
                    .child(
                        svg()
                            .path(icons::path("icons/star-filled.svg"))
                            .absolute()
                            .left_0()
                            .when(rating.is_multiple_of(2), |star| star.left(-half))
                            .top(top)
                            .size(side)
                            .text_color(match rating <= preview {
                                true => ink,
                                false => theme.muted_foreground.opacity(0.3),
                            }),
                    )
                    .when(!busy, |half| {
                        half.cursor_pointer()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_hover(move |over, _, cx| {
                                if *over {
                                    hover.write(cx, Some(selection));
                                }
                            })
                            .tip_label(label, Perch::Above)
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                if let Some(track) = current_track(&track, playback.as_ref(), cx) {
                                    entity.update(cx, |ratings, cx| {
                                        ratings.rate(track, selection, cx)
                                    });
                                }
                            })
                    })
            }));
        row.style().refine(&overrides);
        row.into_any_element()
    }
}

impl Rating {
    /// The heart, optionally previewing the auto-like result of the hovered rating.
    fn like(&self, saved: Option<bool>, cx: &App) -> Button {
        let track = &self.track;
        let theme = *cx.theme();
        let library = Sonora::global(cx).library.clone();
        let playback = self.playback.clone();
        let held = track.clone();
        let id = track.id.as_deref();
        let saved = saved.unwrap_or_else(|| id.is_some_and(|id| library.read(cx).saved(id)));
        let pending = id.is_some_and(|id| library.read(cx).pending(id));

        Button::new("toggle-liked-track")
            .ghost()
            .backgroundless()
            .small()
            .icon(match saved {
                true => "icons/heart-filled.svg",
                false => "icons/heart.svg",
            })
            .tooltip_above(match saved {
                true => "menu-remove-from-library",
                false => "menu-add-to-library",
            })
            .tint(match saved {
                true => theme.primary,
                false => theme.muted_foreground,
            })
            .disabled(id.is_none() || pending)
            .when(self.frosted, Button::frosted)
            .on_click(move |_, _, cx| {
                if let Some(track) = current_track(&held, playback.as_ref(), cx) {
                    library.update(cx, |library, cx| library.toggle(track, cx));
                }
            })
    }
}

/// Player controls follow the displayed track at click time, including another device's playback.
fn current_track(track: &Track, playback: Option<&Entity<Playback>>, cx: &App) -> Option<Track> {
    match playback {
        Some(playback) => playback.read(cx).shown_track(cx),
        None => Some(track.clone()),
    }
}
