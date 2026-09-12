use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla, Interactivity, MouseButton,
    MouseDownEvent, Pixels, SharedString, Stateful, StyleRefinement, Window, div, px, relative,
    svg,
};

use crate::ExplicitBadge;
use crate::artwork::{Artwork, Avatar, ROUNDED};
use crate::button::Button;
use crate::label::upper;
use crate::metrics::{LEADING, Text, snapped};
use crate::skeleton::Skeleton;
use crate::theme::ActiveTheme as _;
use crate::tooltip::{Perch, Tooltip};

const BAR_TITLE: (Pixels, Pixels) = (px(140.), px(11.));
const BAR_META: (Pixels, Pixels) = (px(90.), px(9.));
const PLAY_RATIO: f32 = 0.24;
const PLAY_MIN: Pixels = px(20.);
const PLAY_MAX: Pixels = px(40.);
const PLAY_INSET: Pixels = px(8.);
const SCRIM_RATIO: f32 = 0.45;
const SCRIM_MIN: Pixels = px(14.);
const GLYPH_RATIO: f32 = 0.34;
const GLYPH_MIN: Pixels = px(12.);
/// How far the artwork under a glyph is dimmed, so the glyph reads over any cover.
const GLYPH_VEIL: f32 = 0.45;
const TIGHT: Pixels = px(2.);

pub const CARD_GROUP: &str = "card";

type Press = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type Summon = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;
type DragStart = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Card {
    base: Stateful<Div>,
    title: SharedString,
    eyebrow: Option<SharedString>,
    size: Option<Text>,
    weight: Option<FontWeight>,
    meta: Option<AnyElement>,
    bare: bool,
    trailing: Option<AnyElement>,
    cover: Option<String>,
    fallback: Option<SharedString>,
    glyph: Option<SharedString>,
    accent: bool,
    art: Option<Pixels>,
    art_radius: Option<Pixels>,
    circle: bool,
    tint: Option<Hsla>,
    explicit: bool,
    fill: bool,
    hovered: Option<StyleRefinement>,
    press: Option<Press>,
    loading: bool,
    tile: Option<Pixels>,
    play: Option<Press>,
    hint: bool,
    underline: bool,
    playing: bool,
    chosen: bool,
    action: Option<AnyElement>,
    drag_start: Option<DragStart>,
    menu: Option<Summon>,
}

impl Card {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            base: div().id(id),
            title: title.into(),
            eyebrow: None,
            size: None,
            weight: None,
            meta: None,
            bare: false,
            trailing: None,
            cover: None,
            fallback: None,
            glyph: None,
            accent: false,
            art: None,
            art_radius: None,
            circle: false,
            tint: None,
            explicit: false,
            fill: true,
            hovered: None,
            press: None,
            loading: false,
            tile: None,
            play: None,
            hint: false,
            underline: false,
            playing: false,
            chosen: false,
            action: None,
            drag_start: None,
            menu: None,
        }
    }

    pub fn skeleton(id: impl Into<ElementId>) -> Self {
        Self::new(id, "").loading()
    }

    pub fn drag_start(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.drag_start = Some(Rc::new(handler));
        self
    }

    pub fn tile(mut self, width: Pixels) -> Self {
        self.tile = Some(width);
        self
    }

    pub fn tile_height(art: Pixels, window: &Window, cx: &App) -> Pixels {
        let theme = *cx.theme();
        let rem = window.rem_size();
        let line = |step: Text| px((theme.text(step) / px(1.) * LEADING).round());

        snapped(
            art + rem * 0.5 + line(Text::Body) + TIGHT + line(Text::Small),
            window,
        )
    }

    pub fn play(
        mut self,
        playing: bool,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.play = Some(Box::new(handler));
        self.playing = playing;
        self
    }

    pub fn hint(mut self) -> Self {
        self.hint = true;
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub fn cover(mut self, cover: Option<String>) -> Self {
        self.cover = cover;
        self
    }

    pub fn fallback(mut self, icon: impl Into<SharedString>) -> Self {
        self.fallback = Some(icon.into());
        self
    }

    /// A glyph resting on the artwork under a light veil. It says what the card stands for when
    /// the picture alone cannot: a folder among the playlists it sits beside.
    pub fn glyph(mut self, icon: impl Into<SharedString>) -> Self {
        self.glyph = Some(icon.into());
        self
    }

    pub fn accent(mut self) -> Self {
        self.accent = true;
        self
    }

    pub fn art(mut self, art: Pixels) -> Self {
        self.art = Some(art);
        self
    }

    pub fn art_radius(mut self, radius: Pixels) -> Self {
        self.art_radius = Some(radius);
        self
    }

    pub fn circle(mut self) -> Self {
        self.circle = true;
        self
    }

    pub fn eyebrow(mut self, eyebrow: impl Into<SharedString>) -> Self {
        self.eyebrow = Some(eyebrow.into());
        self
    }

    pub fn size(mut self, size: Text) -> Self {
        self.size = Some(size);
        self
    }

    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = Some(weight);
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    pub fn meta(mut self, meta: impl IntoElement) -> Self {
        self.meta = Some(meta.into_any_element());
        self
    }

    pub fn bare_meta(mut self, meta: impl IntoElement) -> Self {
        self.meta = Some(meta.into_any_element());
        self.bare = true;
        self
    }

    pub fn explicit(mut self) -> Self {
        self.explicit = true;
        self
    }

    pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
        self.trailing = Some(trailing.into_any_element());
        self
    }

    pub fn flat(mut self) -> Self {
        self.fill = false;
        self
    }

    pub fn loading(mut self) -> Self {
        self.loading = true;
        self.fill = false;
        self
    }

    pub fn press(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.press = Some(Box::new(handler));
        self
    }

    pub fn menu(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.menu = Some(Box::new(handler));
        self
    }

    pub fn chosen(mut self, chosen: bool) -> Self {
        self.chosen = chosen;
        self
    }
}

impl Styled for Card {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Card {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn hover(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.hovered = Some(f(self.hovered.take().unwrap_or_default()));
        self
    }
}

impl StatefulInteractiveElement for Card {}

impl RenderOnce for Card {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            title,
            eyebrow,
            size,
            weight,
            meta,
            bare,
            trailing,
            cover,
            fallback,
            glyph,
            accent,
            art,
            art_radius,
            circle,
            tint,
            explicit,
            fill,
            hovered,
            press,
            loading,
            tile,
            play,
            hint,
            underline,
            playing,
            chosen,
            action,
            drag_start,
            menu,
        } = self;

        let theme = *cx.theme();
        let inset = theme.metrics.pad;
        let height = snapped(theme.metrics.list_row, window);
        let listed = art.is_none() && tile.is_none();
        let art_radius = art_radius.or_else(|| tile.map(|_| theme.radius));
        let art = art.or(tile).unwrap_or(snapped(height - inset * 2., window));
        let hovered = match (hovered, fill) {
            (Some(style), _) => Some(style),
            (None, true) => Some(StyleRefinement::default().bg(theme.table_hover)),
            (None, false) => None,
        };
        let overrides = std::mem::take(base.style());

        let leading = match loading {
            true => Skeleton::new()
                .size(art)
                .when(circle, Skeleton::circle)
                .into_any_element(),
            false if circle => Avatar::new(cover).size(art).into_any_element(),
            false => Artwork::new(cover)
                .size(art)
                .when_some(art_radius, Artwork::corner_radius)
                .when_some(fallback, Artwork::fallback)
                .when(accent, Artwork::accent)
                .into_any_element(),
        };
        let corner = match (circle, art_radius) {
            (true, _) => art / 2.,
            (false, Some(radius)) => radius,
            (false, None) => theme.radius.min(ROUNDED),
        };
        let leading = match glyph {
            None => leading,
            Some(glyph) => {
                let size = px((art / px(1.) * GLYPH_RATIO).round()).max(GLYPH_MIN);

                div()
                    .relative()
                    .flex_none()
                    .size(art)
                    .child(leading)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(corner)
                            .bg(theme.overlay.opacity(GLYPH_VEIL))
                            .child(
                                svg()
                                    .path(icons::path(glyph))
                                    .size(size)
                                    .flex_none()
                                    .text_color(theme.overlay_foreground),
                            ),
                    )
                    .into_any_element()
            }
        };
        let leading = match play {
            None => leading,
            Some(play) => {
                let glyph = match playing {
                    true => "icons/pause-filled.svg",
                    false => "icons/play-filled.svg",
                };
                let hint = match playing {
                    true => "play-pause",
                    false => "common-play",
                };
                let control = match tile {
                    Some(_) => {
                        let size =
                            px((art / px(1.) * PLAY_RATIO).round()).clamp(PLAY_MIN, PLAY_MAX);

                        div()
                            .id("card-play")
                            .absolute()
                            .right(PLAY_INSET)
                            .bottom(PLAY_INSET)
                            .when(!playing, |this| {
                                this.invisible()
                                    .group_hover(CARD_GROUP, |style| style.visible())
                            })
                            .child(
                                Button::new("card-play-button")
                                    .primary()
                                    .icon(glyph)
                                    .tooltip(hint)
                                    .size(size)
                                    .rounded_full()
                                    .shadow_sm()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(move |event, window, cx| {
                                        cx.stop_propagation();
                                        play(event, window, cx);
                                    }),
                            )
                            .into_any_element()
                    }
                    None => {
                        let size = px((art / px(1.) * SCRIM_RATIO).round()).max(SCRIM_MIN);

                        div()
                            .id("card-scrim")
                            .absolute()
                            .inset_0()
                            .when(!playing, |this| {
                                this.invisible()
                                    .group_hover(CARD_GROUP, |style| style.visible())
                            })
                            .child(
                                div()
                                    .id("card-play")
                                    .size_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(corner)
                                    .cursor_pointer()
                                    .bg(theme.overlay)
                                    .tooltip(Tooltip::build(hint, Perch::Pointer))
                                    .child(
                                        svg()
                                            .path(icons::path(glyph))
                                            .size(size)
                                            .flex_none()
                                            .text_color(theme.overlay_foreground),
                                    )
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(move |event, window, cx| {
                                        cx.stop_propagation();
                                        play(event, window, cx);
                                    }),
                            )
                            .into_any_element()
                    }
                };

                div()
                    .relative()
                    .flex_none()
                    .size(art)
                    .child(leading)
                    .child(control)
                    .into_any_element()
            }
        };

        let title = div()
            .id("card-title")
            .min_w_0()
            .truncate()
            .when_some(drag_start.clone(), |this, drag_start| {
                this.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                    drag_start(event, window, cx)
                })
            })
            .when_some(weight, |this, weight| this.font_weight(weight))
            .when(underline, |this| this.hover(|style| style.underline()))
            .when(hint && !title.is_empty(), |this| {
                this.tooltip(Tooltip::label(title.clone(), Perch::Pointer))
            })
            .text_color(tint.unwrap_or(theme.foreground))
            .when_some(size, |this, size| this.text_size(theme.text(size)))
            .child(title);
        let heading = match explicit {
            false => title.into_any_element(),
            true => div()
                .flex()
                .items_center()
                .gap(px(3.))
                .min_w_0()
                .child(title)
                .child(div().flex_none().child(ExplicitBadge::new()))
                .into_any_element(),
        };
        let caption = meta.map(|meta| match bare {
            true => div().child(meta),
            false => div()
                .min_w_0()
                .truncate()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(meta),
        });

        let mut card = base
            .group(CARD_GROUP)
            .flex()
            .when_else(
                tile.is_some(),
                |this| this.flex_col().gap_2().w(art),
                |this| this.items_center().gap_3().px(inset),
            )
            .rounded(theme.radius)
            .when(listed, |this| {
                this.flex_none().h(height).py(inset).w_full().min_w_0()
            })
            .when(chosen, |this| this.bg(theme.table_active))
            .when_some(hovered.filter(|_| !chosen), |this, style| {
                this.hover(move |_| style)
            })
            .when_some(press, |this, press| {
                this.cursor_pointer()
                    .on_click(move |event, window, cx| press(event, window, cx))
            })
            .when_some(menu, |this, menu| {
                this.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    menu(event, window, cx);
                })
            })
            .child(match drag_start.clone() {
                Some(drag_start) => div()
                    .flex_none()
                    .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                        drag_start(event, window, cx)
                    })
                    .child(leading)
                    .into_any_element(),
                None => leading,
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .line_height(relative(LEADING))
                    .when(tile.is_some(), |this| this.w_full().flex_none())
                    .when_else(
                        loading,
                        |this| {
                            this.gap_2()
                                .child(Skeleton::new().w(BAR_TITLE.0).h(BAR_TITLE.1))
                                .child(Skeleton::new().w(BAR_META.0).h(BAR_META.1))
                        },
                        |this| match eyebrow {
                            None => this.gap(TIGHT).child(heading).children(caption),
                            Some(eyebrow) => this
                                .when(tile.is_some(), |this| this.gap_1())
                                .child(
                                    div()
                                        .text_size(theme.text(Text::Small))
                                        .text_color(theme.muted_foreground)
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(upper(eyebrow)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w_0()
                                        .gap(TIGHT)
                                        .child(heading)
                                        .children(caption),
                                ),
                        },
                    ),
            )
            .children(trailing.map(|trailing| div().flex_none().child(trailing)))
            .children(action.map(|action| {
                div()
                    .id("card-action")
                    .flex_none()
                    .invisible()
                    .group_hover(CARD_GROUP, |style| style.visible())
                    .child(action)
            }));

        card.style().refine(&overrides);
        card
    }
}
