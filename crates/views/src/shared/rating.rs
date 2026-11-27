use std::rc::Rc;

use gpui::accesskit::{Action, Role};
use gpui::prelude::*;
use gpui::{
    App, Div, ElementId, Entity, Hsla, MouseButton, Pixels, StyleRefinement, Window, div, px, svg,
};
use i18n::t;
use input::{ActivateRating, RATING_CONTEXT};
use music::Track;
use state::{Playback, Sonora};
use ui::{ActiveTheme as _, Button, Metrics, Perch, Tipped as _};

/// Star size as a fraction of its hit height.
const MARK: f32 = 0.62;
/// Opacity of an empty star.
const EMPTY: f32 = 0.3;
/// Opacity while ratings load.
const BUSY: f32 = 0.5;

/// Five stars with a heart, or a cross for the auto-like threshold.
#[derive(IntoElement)]
pub(crate) struct Rating {
    base: Div,
    id: ElementId,
    track: Option<Track>,
    playback: Option<Entity<Playback>>,
    frosted: bool,
    disabled: bool,
    hover: Option<Entity<Option<u8>>>,
}

/// The rating control for `track`, if any.
pub(crate) fn rating(
    id: impl Into<ElementId>,
    track: Option<&Track>,
    playback: Option<&Entity<Playback>>,
    frosted: bool,
) -> Option<Rating> {
    track.map(|track| Rating {
        id: id.into(),
        track: Some(track.clone()),
        playback: playback.cloned(),
        frosted,
        ..Rating::threshold()
    })
}

impl Styled for Rating {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Rating {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let stars_only = self.hover.is_some();
        let hovered = self.hover.clone().unwrap_or_else(|| {
            window.use_keyed_state((self.id.clone(), "hover"), cx, |_, _| None::<u8>)
        });
        let sonora = Sonora::global(cx);
        let entity = sonora.ratings.clone();
        let library = sonora.library.clone();
        let settings = sonora.settings.clone();
        let ratings = entity.read(cx);
        let options = settings.read(cx).star_ratings();
        let id = self.track.as_ref().and_then(|track| track.id.as_deref());
        let threshold = self.track.is_none();
        let active = threshold
            || self
                .track
                .as_ref()
                .is_some_and(|track| ratings.can_rate(track, cx));
        let halves = ratings.halves(cx);
        let hearts = options.hearts || threshold;
        let busy = id.is_some_and(|id| ratings.loading() || ratings.pending(id));
        let blocked = busy || self.disabled;
        let value = match threshold {
            true => options.auto_like,
            false => id.and_then(|id| ratings.for_track(id)),
        };
        let hovering = (*hovered.read(cx)).filter(|_| active && !blocked);
        let saved = match threshold {
            true => value.is_none(),
            false => id
                .and_then(|id| ratings.preview_like(id, hovering))
                .unwrap_or_else(|| id.is_some_and(|id| library.read(cx).saved(id))),
        };
        let disabled = !threshold && id.is_none_or(|id| library.read(cx).pending(id));
        let failed = !threshold && ratings.failed();
        let track = self.track.clone();
        let playback = self.playback.clone();
        // One action for every star and the heart.
        let selected_ratings = entity.clone();
        let select = Rc::new(move |value: Option<u8>, cx: &mut App| {
            let Some(track) = track.as_ref() else {
                settings.update(cx, |settings, cx| {
                    settings.set_star_ratings(|ratings| ratings.auto_like = value, cx)
                });
                return;
            };
            let track = match playback.as_ref() {
                Some(playback) => playback.read(cx).shown_track(cx),
                None => Some(track.clone()),
            };
            let Some(track) = track else { return };
            match value {
                Some(value) => {
                    selected_ratings.update(cx, |ratings, cx| ratings.rate(track, value, cx))
                }
                None => library.update(cx, |library, cx| library.toggle(track, cx)),
            }
        });
        let theme = *cx.theme();
        let (icon, tip) = match (threshold, saved) {
            (true, _) => ("icons/x.svg", "settings-rating-auto-like-off"),
            (false, true) => ("icons/heart-filled.svg", "menu-remove-from-library"),
            (false, false) => ("icons/heart.svg", "menu-add-to-library"),
        };
        let press = select.clone();
        let off = select.clone();
        let mut leading = Button::new(match threshold {
            true => ElementId::from("rating-auto-like-off"),
            false => ElementId::from((self.id.clone(), "liked")),
        })
        .ghost()
        .backgroundless()
        .small()
        .icon(icon)
        .tooltip_above(tip)
        .tint(match saved {
            true => theme.primary,
            false => theme.muted_foreground,
        })
        .disabled(disabled || self.disabled)
        .when(self.frosted, Button::frosted)
        .on_click(move |_, _, cx| press(None, cx))
        .when(threshold && !blocked, |button| {
            keyboard(button, theme.secondary_active, true, move |cx| {
                off(None, cx)
            })
        });
        if !active {
            leading.style().refine(&std::mem::take(self.base.style()));
            return match stars_only || !hearts {
                true => div().into_any_element(),
                false => leading.into_any_element(),
            };
        }
        let label = |selection| match (threshold, value == Some(selection)) {
            (true, _) => t!(
                "settings-rating-auto-like-set",
                value = f64::from(selection) / 2.
            ),
            (false, true) => t!("rating-clear"),
            (false, false) => t!("rating-set", rating = f64::from(selection) / 2.),
        };
        let preview = hovering.or(value).unwrap_or(0);
        let (hit, half, gap) = star_metrics(theme.metrics);
        let half = ui::snapped(half, window);
        let side = half * 2.;
        let top = ui::snapped((hit - side) / 2., window);
        let ink = hovering.map_or(theme.foreground, |_| theme.primary);
        let reset = hovered.clone();
        let overrides = std::mem::take(self.base.style());
        let row = self
            .base
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            // Without the heart, the stars take its inset.
            .map(|row| match (stars_only, hearts) {
                (false, true) => row.child(div().mr(gap).child(leading)),
                (false, false) => row.pl(top),
                (true, _) => row,
            });
        if failed {
            let retry = move |cx: &mut App| entity.update(cx, |ratings, cx| ratings.retry(cx));
            let press = retry.clone();
            let button = Button::new("rating-retry")
                .ghost()
                .small()
                .icon("icons/refresh-cw.svg")
                .tooltip_above("rating-retry")
                .on_click(move |_, _, cx| press(cx));
            let mut row = row.child(keyboard(button, theme.secondary_active, true, retry));
            row.style().refine(&overrides);
            return row.into_any_element();
        }

        let mut row = row
            .on_hover(move |over, _, cx| {
                if !over {
                    reset.write(cx, None);
                }
            })
            .when(busy, |row| {
                row.opacity(BUSY).tip("rating-loading", Perch::Above)
            })
            .children((1..=5u8).map(|star| {
                // Each star is one shape per colour, clipped where its fill ends, so no seam
                // shows inside it. The two halves on top only take clicks.
                let empty = theme.muted_foreground.opacity(EMPTY);
                let pieces = match preview.saturating_sub(2 * (star - 1)).min(2) {
                    0 => vec![(empty, Pixels::ZERO, side)],
                    1 => vec![(ink, Pixels::ZERO, half), (empty, half, half)],
                    _ => vec![(ink, Pixels::ZERO, side)],
                };
                let zones = [2 * star - 1, 2 * star].map(|rating| {
                    let selection = match halves {
                        true => rating,
                        false => rating.next_multiple_of(2),
                    };
                    let label = label(selection);
                    let hover = hovered.clone();
                    let select = select.clone();
                    let accessible = select.clone();
                    let activate = select.clone();
                    div()
                        .id(("rating", usize::from(rating)))
                        .role(Role::Button)
                        .aria_label(label.clone())
                        .absolute()
                        .top_0()
                        .w(half)
                        .h(hit)
                        .map(|zone| match rating.is_multiple_of(2) {
                            true => zone.right_0(),
                            false => zone.left_0(),
                        })
                        .when(halves || rating == selection, |zone| {
                            keyboard(zone, theme.secondary_active, !blocked, move |cx| {
                                activate(Some(selection), cx)
                            })
                        })
                        .when(!blocked, |zone| {
                            zone.cursor_pointer()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_hover(move |over, _, cx| {
                                    if *over {
                                        hover.write(cx, Some(selection));
                                    }
                                })
                                .tip_label(label, Perch::Above)
                                .on_a11y_action(Action::Click, move |_, _, cx| {
                                    accessible(Some(selection), cx)
                                })
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    select(Some(selection), cx);
                                })
                        })
                });
                div()
                    .relative()
                    .flex_none()
                    .w(side)
                    .h(hit)
                    .when(star > 1, |this| this.ml(gap))
                    .children(pieces.into_iter().map(|(color, from, width)| {
                        div()
                            .absolute()
                            .left(from)
                            .top_0()
                            .w(width)
                            .h(hit)
                            .overflow_hidden()
                            .child(
                                svg()
                                    .path(icons::path("icons/star-filled.svg"))
                                    .absolute()
                                    .left(-from)
                                    .top(top)
                                    .size(side)
                                    .text_color(color),
                            )
                    }))
                    .children(zones)
            }));
        row.style().refine(&overrides);
        row.into_any_element()
    }
}

impl Rating {
    /// The auto-like threshold control.
    pub(crate) fn threshold() -> Self {
        Self {
            base: div(),
            id: "rating-auto-like".into(),
            track: None,
            playback: None,
            frosted: false,
            disabled: false,
            hover: None,
        }
    }

    /// Disables the control while its setting is unavailable.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Shows only stars, sharing hover with a heart tile.
    pub(crate) fn stars_only(mut self, hover: Entity<Option<u8>>) -> Self {
        self.hover = Some(hover);
        self
    }
}

/// The rating control's width at the current settings.
pub(crate) fn width(cx: &App) -> Pixels {
    let sonora = Sonora::global(cx);
    let stars = sonora.ratings.read(cx).enabled(cx);
    let hearts = sonora.settings.read(cx).star_ratings().hearts;
    measure(cx.theme().metrics, hearts, stars)
}

/// The widest the control gets, for table columns.
pub(crate) fn span(metrics: Metrics) -> Pixels {
    measure(metrics, true, true)
}

fn measure(metrics: Metrics, hearts: bool, stars: bool) -> Pixels {
    let (hit, half, gap) = star_metrics(metrics);
    let stars_width = half * 10.;
    match (hearts, stars) {
        (true, true) => hit + gap + stars_width + gap * 4.,
        (false, true) => hit * (1. - MARK) / 2. + stars_width + gap * 4.,
        (true, false) => hit,
        (false, false) => px(0.),
    }
}

/// Hit height, half-star width and gap, before snapping.
fn star_metrics(metrics: Metrics) -> (Pixels, Pixels, Pixels) {
    let hit = metrics.control_small;
    (hit, hit * MARK / 2., metrics.pad / 4.)
}

fn keyboard<E: StatefulInteractiveElement>(
    element: E,
    focus_fill: Hsla,
    enabled: bool,
    activate: impl Fn(&mut App) + 'static,
) -> E {
    element
        .tab_index(0)
        .tab_stop(enabled)
        .key_context(RATING_CONTEXT)
        .focus_visible(|style| style.bg(focus_fill))
        .on_action(move |_: &ActivateRating, _, cx| {
            if enabled {
                activate(cx);
            }
        })
}
