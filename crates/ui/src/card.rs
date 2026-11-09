//! The card an album, artist or playlist shows as: its art, its title and a muted line under it,
//! as a list row or as a tile in a grid, with a play button the pointer brings out.

use rune::{Cursor, El, PointerPress, TextStyle, Tx, col, icon, row, text};

use crate::button::Button;
use crate::explicit::ExplicitBadge;
use crate::label::{Weight, upper};
use crate::metrics::{LEADING, Text, tucked};
use crate::motion::{PRESS_SCALE_CARD, Springs};
use crate::paint::{Press, rems, shadow_sm};
use crate::skeleton::Skeleton;
use crate::theme::{CoverFill, Theme};
use crate::color::Hsla;

/// The loading bars in place of a title and a meta line.
const BAR_TITLE: (f32, f32) = (140., 11.);
const BAR_META: (f32, f32) = (90., 9.);
/// The tile's play button against its art, within these sizes.
const PLAY_RATIO: f32 = 0.24;
const PLAY_MIN: f32 = 20.;
const PLAY_MAX: f32 = 40.;
const PLAY_INSET: f32 = 8.;
/// How much of the accent colour the tile's play button keeps over the art.
const PLAY_FILL: f32 = 0.55;
/// The list row's play glyph against its art, at least this big.
const SCRIM_RATIO: f32 = 0.45;
const SCRIM_MIN: f32 = 14.;
/// The corners art takes in a list row before the theme's radius.
const ROUNDED: f32 = 4.;
/// The space between a title and its meta line.
const TIGHT: f32 = 2.;
/// The space between a title and the explicit badge after it.
const BADGE_GAP: f32 = 3.;

/// What the right-click handler does, held until the card is built.
type Summon = Box<dyn FnOnce(El) -> El>;

/// A card. Pass its art with [`Card::art`] (a built cover or avatar of the right size) and its
/// handlers through the builder; the built element is the card itself, keyed, a button named
/// for its title when it opens something and a group otherwise.
pub struct Card {
    key: String,
    title: String,
    eyebrow: Option<String>,
    /// What a screen reader reads after the title, standing in for the meta line.
    description: Option<String>,
    size: Option<Text>,
    weight: Option<f32>,
    meta: Option<El>,
    trailing: Option<El>,
    art: Option<El>,
    art_size: Option<f32>,
    art_radius: Option<f32>,
    circle: bool,
    tint: Option<Hsla>,
    explicit: bool,
    fill: bool,
    hover: Option<Hsla>,
    press: Press,
    loading: bool,
    tile: Option<f32>,
    play: Option<(bool, Press)>,
    play_fill: Option<CoverFill>,
    hint: bool,
    chosen: bool,
    tucked: bool,
    action: Option<El>,
    menu: Option<Summon>,
}

impl Card {
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            eyebrow: None,
            description: None,
            size: None,
            weight: None,
            meta: None,
            trailing: None,
            art: None,
            art_size: None,
            art_radius: None,
            circle: false,
            tint: None,
            explicit: false,
            fill: true,
            hover: None,
            press: Press::default(),
            loading: false,
            tile: None,
            play: None,
            play_fill: None,
            hint: false,
            chosen: false,
            tucked: false,
            action: None,
            menu: None,
        }
    }

    /// A card still loading: a pulsing block in place of the art and bars in place of the text.
    pub fn skeleton(key: impl Into<String>) -> Self {
        Self::new(key, "").loading()
    }

    /// A tile `width` wide, its art square over the text, for a grid.
    pub fn tile(mut self, width: f32) -> Self {
        self.tile = Some(width);
        self
    }

    /// How tall a tile with `art` of art stands, text included, for a grid sizing its rows.
    pub fn tile_height(art: f32, theme: &Theme) -> f32 {
        let line = |step: Text| (theme.text(step) * LEADING).round();
        (art + rems(theme, 0.5) + line(Text::Body) + TIGHT + line(Text::Small)).round()
    }

    /// The card's art, already built at [`Card::art_size`] (or the row's or tile's size), such
    /// as `Cover::new(&artwork, url).size(side).build(theme)`.
    pub fn art(mut self, art: El) -> Self {
        self.art = Some(art);
        self
    }

    /// The art's side, when it differs from what a row or tile gives it.
    pub fn art_size(mut self, side: f32) -> Self {
        self.art_size = Some(side);
        self
    }

    pub fn art_radius(mut self, radius: f32) -> Self {
        self.art_radius = Some(radius);
        self
    }

    /// Round art, for an artist.
    pub fn circle(mut self) -> Self {
        self.circle = true;
        self
    }

    /// A play button over the art that the pointer brings out, showing pause while `playing`
    /// and staying out while it does.
    pub fn play<S: 'static>(mut self, playing: bool, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        let mut press = Press::default();
        press.press_tx(f);
        self.play = Some((playing, press));
        self
    }

    /// The colours of the tile's play button, from the cover's palette. The theme's primary
    /// until the palette is known.
    pub fn play_fill(mut self, fill: CoverFill) -> Self {
        self.play_fill = Some(fill);
        self
    }

    /// Shows the title as a tooltip, for a title the card may cut short.
    pub fn hint(mut self) -> Self {
        self.hint = true;
        self
    }

    /// An element the pointer brings out at the card's end, such as a menu button.
    pub fn action(mut self, action: El) -> Self {
        self.action = Some(action);
        self
    }

    pub fn eyebrow(mut self, eyebrow: impl Into<String>) -> Self {
        self.eyebrow = Some(eyebrow.into());
        self
    }

    /// What a screen reader reads after the title, such as the artist and year the meta line
    /// shows. The eyebrow stands in when this is not set.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn size(mut self, size: Text) -> Self {
        self.size = Some(size);
        self
    }

    pub fn weight(mut self, weight: f32) -> Self {
        self.weight = Some(weight);
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    /// The muted line under the title, small and cut short where it runs out of room.
    pub fn meta(mut self, meta: impl Into<String>, theme: &Theme) -> Self {
        self.meta = Some(
            text(meta)
                .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .truncate()
                .shrink()
                .min_w(0.)
                .a11y_hidden(),
        );
        self
    }

    /// An element under the title in place of the muted line, drawn as given.
    pub fn bare_meta(mut self, meta: El) -> Self {
        self.meta = Some(meta);
        self
    }

    pub fn explicit(mut self) -> Self {
        self.explicit = true;
        self
    }

    pub fn trailing(mut self, trailing: El) -> Self {
        self.trailing = Some(trailing);
        self
    }

    /// No hover fill.
    pub fn flat(mut self) -> Self {
        self.fill = false;
        self
    }

    /// The hover fill in place of the table hover.
    pub fn hover(mut self, color: Hsla) -> Self {
        self.hover = Some(color);
        self
    }

    /// Rounds the card to sit inside a padded panel such as a dropdown, so its corners follow
    /// the panel's.
    pub fn tucked(mut self) -> Self {
        self.tucked = true;
        self
    }

    pub fn loading(mut self) -> Self {
        self.loading = true;
        self.fill = false;
        self
    }

    pub fn on_press<S: 'static>(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.press.press(f);
        self
    }

    pub fn on_press_tx<S: 'static>(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.press.press_tx(f);
        self
    }

    /// Opens a context menu on a secondary press, at the window point it landed on.
    pub fn menu<S: 'static>(
        mut self,
        f: impl Fn(&mut S, PointerPress, &mut Tx) + 'static,
    ) -> Self {
        self.menu = Some(Box::new(move |e: El| e.on_secondary_press(f)));
        self
    }

    /// Paints the card as chosen, as the current page's entry in a list.
    pub fn chosen(mut self, chosen: bool) -> Self {
        self.chosen = chosen;
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let inset = theme.metrics.pad;
        let height = theme.metrics.list_row.round();
        let listed = self.art_size.is_none() && self.tile.is_none();
        let art = self
            .art_size
            .or(self.tile)
            .unwrap_or((height - inset * 2.).round());
        let art_radius = self.art_radius.or(self.tile.map(|_| theme.radius));
        let corner = match (self.circle, art_radius) {
            (true, _) => art / 2.,
            (false, Some(radius)) => radius,
            (false, None) => theme.radius.min(ROUNDED),
        };
        let pressable = !self.press.is_empty() && !self.loading;

        let leading = match (self.loading, self.art) {
            (true, _) | (false, None) => {
                let skeleton = match self.circle {
                    true => Skeleton::new().circle(),
                    false => Skeleton::new(),
                };
                skeleton.build(theme).size(art, art).radius(corner)
            }
            (false, Some(art)) => art,
        };
        let leading = match self.play {
            None => leading,
            Some((playing, press)) => {
                let glyph = match playing {
                    true => "icons/pause-filled.svg",
                    false => "icons/play-filled.svg",
                };
                let hint = match playing {
                    true => i18n::t!("play-pause"),
                    false => i18n::t!("common-play"),
                };
                let reveal = |e: El| match playing {
                    true => e,
                    false => e.group_opacity(0., 1.),
                };
                let control = match self.tile {
                    Some(_) => {
                        let side = (art * PLAY_RATIO).round().clamp(PLAY_MIN, PLAY_MAX);
                        let fill = self.play_fill.unwrap_or_else(|| theme.cover_fill(None));
                        let button = Button::new("play")
                            .primary()
                            .icon(glyph)
                            .tooltip(hint)
                            .fill(fill.background.opacity(PLAY_FILL), fill.hover)
                            .tint(fill.foreground)
                            .build(theme)
                            .size(side, side)
                            .px(0.)
                            .radius(side / 2.);
                        let button = shadow_sm(press.apply(button, true));
                        reveal(
                            row()
                                .key("play-corner")
                                .pin(None, None, Some(PLAY_INSET), Some(PLAY_INSET))
                                .child(button),
                        )
                    }
                    None => {
                        let side = (art * SCRIM_RATIO).round().max(SCRIM_MIN);
                        let scrim = row()
                            .key("play")
                            .role("button")
                            .label(hint.clone())
                            .help(hint)
                            .focusable(true)
                            .absolute(0., 0.)
                            .size(art, art)
                            .center()
                            .radius(corner)
                            .fill(theme.overlay.color())
                            .cursor(Cursor::Pointer)
                            .child(
                                icon(icons::get(glyph), side)
                                    .color(theme.overlay_foreground.color())
                                    .a11y_hidden(),
                            );
                        reveal(press.apply(scrim, true))
                    }
                };
                row()
                    .key("art")
                    .size(art, art)
                    .child(leading)
                    .child(control)
            }
        };

        // The meta line is an element no reader can reach, so the description or the eyebrow
        // stands in for it, and the explicit badge is read after them.
        let about = [
            self.description.clone().or_else(|| self.eyebrow.clone()),
            self.explicit
                .then(|| i18n::t!("song-explicit").into_owned()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");

        let words = match self.loading {
            true => col()
                .gap(rems(theme, 0.5))
                .child(Skeleton::new().build(theme).size(BAR_TITLE.0, BAR_TITLE.1))
                .child(Skeleton::new().build(theme).size(BAR_META.0, BAR_META.1)),
            false => {
                let style = TextStyle::new(
                    theme.text(self.size.unwrap_or(Text::Body)),
                    self.weight.unwrap_or(Weight::NORMAL),
                );
                let mut title = text(self.title.clone())
                    .key("title")
                    .font(style)
                    .color(self.tint.unwrap_or(theme.foreground).color())
                    .truncate()
                    .shrink()
                    .min_w(0.)
                    .a11y_hidden();
                if self.hint && !self.title.is_empty() {
                    title = title.help_when_truncated(self.title.clone());
                }
                let heading = match self.explicit {
                    false => title,
                    true => row()
                        .center_y()
                        .gap(BADGE_GAP)
                        .min_w(0.)
                        .child(title)
                        .child(ExplicitBadge::new().build(theme)),
                };
                let stack = col()
                    .min_w(0.)
                    .gap(TIGHT)
                    .child(heading)
                    .children(self.meta);
                match self.eyebrow {
                    None => stack,
                    Some(eyebrow) => col()
                        .min_w(0.)
                        .gap(match self.tile {
                            Some(_) => rems(theme, 0.25),
                            None => 0.,
                        })
                        .child(
                            text(upper(eyebrow))
                                .font(TextStyle::new(theme.text(Text::Small), Weight::SEMIBOLD))
                                .color(theme.muted_foreground.color())
                                .a11y_hidden(),
                        )
                        .child(stack),
                }
            }
        };
        let words = match self.tile {
            Some(_) => words.w_full(),
            None => words.grow(1.).shrink().min_w(0.),
        };

        let radius = match self.tucked {
            true => tucked(theme.radius, theme.font_size),
            false => theme.radius,
        };
        let mut card = match self.tile {
            Some(width) => col().gap(rems(theme, 0.5)).w(width),
            None => row().center_y().gap(rems(theme, 0.75)).px(inset),
        }
        .key(self.key)
        .role(match pressable {
            true => "button",
            false => "group",
        })
        .label(self.title)
        .radius(radius)
        .hover_group_focus()
        .motion(Springs::CONTROL);
        if listed {
            card = card.h(height).py(inset).w_full().min_w(0.);
        }
        if !about.is_empty() {
            card = card.description(about);
        }
        if self.chosen {
            card = card.fill(theme.table_active.color()).focus_selected(true);
        } else if let Some(hover) = self.hover.or(self.fill.then_some(theme.table_hover)) {
            card = card.hover(hover.color());
        }
        if pressable {
            card = card
                .focusable(true)
                .cursor(Cursor::Pointer)
                .press_scale(PRESS_SCALE_CARD);
        }
        card = card.child(leading).child(words).children(self.trailing);
        if let Some(action) = self.action {
            card = card.child(row().key("action").group_opacity(0., 1.).child(action));
        }
        if let Some(menu) = self.menu {
            card = menu(card);
        }
        self.press.apply(card, pressable)
    }
}
