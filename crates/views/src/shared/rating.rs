use std::rc::Rc;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{App, Div, ElementId, Entity, MouseButton, StyleRefinement, Window, div, svg};
use i18n::t;
use music::Track;
use state::{Playback, Sonora};
use ui::{ActiveTheme as _, Button, Perch, Tipped as _};

/// The star's size as a fraction of its hit area's height.
const MARK: f32 = 0.62;
const EMPTY: f32 = 0.3;

/// Five stars with a track's heart, or a cross for the auto-like threshold. Values are half-stars.
#[derive(IntoElement)]
pub(crate) struct Rating {
    base: Div,
    id: ElementId,
    track: Option<Track>,
    playback: Option<Entity<Playback>>,
    frosted: bool,
    hover: Option<Entity<Option<u8>>>,
}

/// Shared by track rows, menus and players. Player clicks act on the track playing at click time.
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
            || (ratings.enabled(cx)
                && id.is_some_and(|id| !id.is_empty() && !music::is_local_id(id)));
        let busy = id.is_some_and(|id| ratings.loading() || ratings.pending(id));
        let value = match threshold {
            true => options.auto_like,
            false => id.and_then(|id| ratings.for_track(id)),
        };
        let hovering = (*hovered.read(cx)).filter(|_| active && !busy);
        let preview = hovering.or(value).unwrap_or(0);
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
        // One action serves every half-star and the leading heart or Off button.
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
            (true, _) => ("icons/x.svg", "ratings-auto-like-off"),
            (false, true) => ("icons/heart-filled.svg", "menu-remove-from-library"),
            (false, false) => ("icons/heart.svg", "menu-add-to-library"),
        };
        let press = select.clone();
        let mut leading = Button::new(match threshold {
            true => "rating-auto-like-off",
            false => "toggle-liked-track",
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
        .disabled(disabled)
        .when(self.frosted, Button::frosted)
        .on_click(move |_, _, cx| press(None, cx));
        if !active {
            leading.style().refine(&std::mem::take(self.base.style()));
            return match stars_only {
                true => div().into_any_element(),
                false => leading.into_any_element(),
            };
        }
        if failed {
            return Button::new(self.id)
                .ghost()
                .small()
                .icon("icons/refresh-cw.svg")
                .tooltip_above("ratings-retry")
                .on_click(move |_, _, cx| entity.update(cx, |ratings, cx| ratings.retry(cx)))
                .into_any_element();
        }
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
            .when(!stars_only, |row| row.child(div().mr(gap).child(leading)))
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
                let label = match (self.track.is_some(), value == Some(selection)) {
                    (false, _) => t!("ratings-auto-like-set", rating = f64::from(selection) / 2.),
                    (true, true) => t!("ratings-clear"),
                    (true, false) => t!("ratings-set", rating = f64::from(selection) / 2.),
                };
                let hover = hovered.clone();
                let select = select.clone();
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
                                false => theme.muted_foreground.opacity(EMPTY),
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
                                select(Some(selection), cx);
                            })
                    })
            }));
        row.style().refine(&overrides);
        row.into_any_element()
    }
}

impl Rating {
    /// Edits the auto-like threshold with the same stars; the cross switches auto-like off.
    pub(crate) fn threshold() -> Self {
        Self {
            base: div(),
            id: "rating-auto-like".into(),
            track: None,
            playback: None,
            frosted: false,
            hover: None,
        }
    }

    /// Shares hover with a separate favourite tile, leaving only stars in this row.
    pub(crate) fn stars_only(mut self, hover: Entity<Option<u8>>) -> Self {
        self.hover = Some(hover);
        self
    }
}
