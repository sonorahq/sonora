use gpui::prelude::*;
use gpui::{
    App, Bounds, Div, Pixels, Point, SharedString, StyleRefinement, Svg, Window, div, px, svg,
};

use crate::artwork::{Artwork, Avatar};
use crate::metrics::Text;
use crate::theme::ActiveTheme as _;

const CHIP: Pixels = px(240.);
const NUDGE: Pixels = px(8.);
const ART: Pixels = px(20.);
const MARKER: Pixels = px(2.);
const SLACK: Pixels = px(10.);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Edge {
    Above,
    Below,
}

pub fn drop_gap(bounds: Bounds<Pixels>, position: Point<Pixels>, index: usize) -> Option<usize> {
    let slack = SLACK.min(bounds.size.height / 2.);
    if !bounds.dilate(slack).contains(&position) {
        return None;
    }

    Some(match position.y < bounds.center().y {
        true => index,
        false => index + 1,
    })
}

/// Whether `position` sits in the middle half of `bounds`, where a drop lands on the row itself
/// rather than in the gap above or below it.
pub fn drop_into(bounds: Bounds<Pixels>, position: Point<Pixels>) -> bool {
    let quarter = bounds.size.height / 4.;
    bounds.contains(&position)
        && position.y > bounds.top() + quarter
        && position.y < bounds.bottom() - quarter
}

pub fn drop_marker(edge: Edge, cx: &App) -> Div {
    let line = div()
        .absolute()
        .left_2()
        .right_2()
        .h(MARKER)
        .rounded_full()
        .bg(cx.theme().primary);

    match edge {
        Edge::Above => line.top_0(),
        Edge::Below => line.bottom_0(),
    }
}

/// The dotted handle at the start of a row that can be dragged to a new place. It only marks the
/// row, so the caller wires the drag itself.
pub fn grip(cx: &App) -> Svg {
    let theme = cx.theme();

    svg()
        .path(icons::path("icons/grip-vertical.svg"))
        .flex_none()
        .size(theme.text(Text::Body))
        .text_color(theme.muted_foreground)
}

#[derive(IntoElement)]
pub struct Ghost {
    base: Div,
    position: Point<Pixels>,
    label: SharedString,
    cover: Option<String>,
    fallback: Option<SharedString>,
    badge: Option<SharedString>,
    art: bool,
    round: bool,
}

impl Ghost {
    pub fn new(position: Point<Pixels>, label: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            position,
            label: label.into(),
            cover: None,
            fallback: None,
            badge: None,
            art: false,
            round: false,
        }
    }

    pub fn art(mut self, cover: Option<String>) -> Self {
        self.cover = cover;
        self.art = true;
        self
    }

    pub fn fallback(mut self, icon: impl Into<SharedString>) -> Self {
        self.fallback = Some(icon.into());
        self
    }

    pub fn circle(mut self) -> Self {
        self.round = true;
        self
    }

    /// An icon after the label that says what the drop does, such as copy or move.
    pub fn badge(mut self, icon: impl Into<SharedString>) -> Self {
        self.badge = Some(icon.into());
        self
    }
}

impl Styled for Ghost {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Ghost {
    fn render(mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let overrides = std::mem::take(self.base.style());
        let leading = self.art.then(|| match self.round {
            true => Avatar::new(self.cover.clone()).size(ART).into_any_element(),
            false => Artwork::new(self.cover.clone())
                .size(ART)
                .corner_radius(theme.radius)
                .when_some(self.fallback.clone(), Artwork::fallback)
                .into_any_element(),
        });

        let mut chip = self
            .base
            .flex()
            .items_center()
            .gap_2()
            .max_w(CHIP)
            .px_2()
            .py_1()
            .rounded(theme.radius)
            .bg(theme.secondary)
            .text_color(theme.foreground)
            .children(leading)
            .child(div().min_w_0().truncate().child(self.label))
            .when_some(self.badge, |chip, badge| {
                chip.child(
                    svg()
                        .path(icons::path(&badge))
                        .flex_none()
                        .size(theme.text(Text::Small))
                        .text_color(theme.muted_foreground),
                )
            });
        chip.style().refine(&overrides);

        div()
            .pl(self.position.x + NUDGE)
            .pt(self.position.y + NUDGE)
            .child(chip)
    }
}
