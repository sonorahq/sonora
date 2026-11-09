//! One option of a visual choice: a thumbnail of what it looks like, its name under it, and a
//! ring round the chosen one. Cards stand side by side in a grid, one per option.

use std::rc::Rc;

use rune::{Cursor, El, TextStyle, Tx, col, el, icon, row, text};

use crate::button::Button;
use crate::label::Weight;
use crate::menu::Menu;
use crate::metrics::Text;
use crate::motion::{PRESS_SCALE_CARD, Springs};
use crate::overlay::Placement;
use crate::paint::{Press, rems};
use crate::popover::{Popover, Popovers};
use crate::theme::Theme;

/// How thick the ring round the chosen card draws, and the gap it leaves in the page colour.
const RING: f32 = 2.;
const RING_GAP: f32 = 2.;
/// How far a card that cannot be chosen fades.
const FADED: f32 = 0.45;
/// How strongly the page shows through a thumbnail at rest.
const FILL: f32 = 0.45;
const BORDER: f32 = 1.;
/// The dashes of a card that adds something rather than choosing.
const DASH: f32 = 4.;
/// The chevron after the name of an option that comes in variants.
const CHEVRON: f32 = 12.;

type Ask<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type Group<S> = Rc<dyn for<'a> Fn(&'a mut S) -> &'a mut Popovers>;

/// What the name of an option that comes in variants does when it is pressed.
enum Variants<S> {
    /// Opens a menu under the name, as the popover `key` of a group.
    Menu {
        key: String,
        group: Popovers,
        access: Group<S>,
        menu: Box<Menu<S>>,
    },
    /// Runs a handler, for a card whose menu another card holds for now.
    Ask(Ask<S>),
}

/// An option card. Under the pointer its thumbnail brightens and its name lights.
pub struct OptionCard<S> {
    key: String,
    thumb: Option<El>,
    label: Option<String>,
    detail: Option<String>,
    variants: Option<Variants<S>>,
    height: Option<f32>,
    selected: bool,
    disabled: bool,
    dashed: bool,
    tooltip: Option<String>,
    press: Press,
    delete: Option<(String, Press)>,
}

impl<S: 'static> OptionCard<S> {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            thumb: None,
            label: None,
            detail: None,
            variants: None,
            height: None,
            selected: false,
            disabled: false,
            dashed: false,
            tooltip: None,
            press: Press::default(),
            delete: None,
        }
    }

    /// What the thumbnail shows. It fills the thumbnail and is cut to its corners.
    pub fn thumb(mut self, thumb: El) -> Self {
        self.thumb = Some(thumb);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A second, smaller line under the label, such as the bitrate a quality streams at.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Puts a chevron after the name and makes the name open `menu` under it as the popover
    /// `key` of `group`, for an option that comes in variants. A press on the name does not
    /// choose the card.
    pub fn variant_menu(
        mut self,
        key: impl Into<String>,
        group: &Popovers,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut Popovers + 'static,
        menu: Menu<S>,
    ) -> Self {
        self.variants = Some(Variants::Menu {
            key: key.into(),
            group: group.clone(),
            access: Rc::new(access),
            menu: Box::new(menu),
        });
        self
    }

    /// Puts the same chevron after the name and runs `f` on a press on it.
    pub fn on_variants(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.variants = Some(Variants::Ask(Rc::new(f)));
        self
    }

    /// How tall the thumbnail stands, one and a half list rows unless set.
    pub fn thumb_height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Draws the thumbnail's edge dashed, for a card that adds or opens something rather than
    /// choosing an option.
    pub fn dashed(mut self) -> Self {
        self.dashed = true;
        self
    }

    /// Text shown while the pointer rests on the card, already resolved. A disabled card keeps
    /// it, so it can say why it cannot be chosen.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn on_press(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.press.press(f);
        self
    }

    /// Puts a trash button in the thumbnail's corner, named by `tooltip`, for an option the user
    /// can remove. It works while the card is disabled too.
    pub fn on_delete(mut self, tooltip: impl Into<String>, f: impl Fn(&mut S) + 'static) -> Self {
        let mut press = Press::default();
        press.press(f);
        self.delete = Some((tooltip.into(), press));
        self
    }

    /// The card, with an open variants menu pushed into `layer`.
    pub fn build(self, theme: &Theme, layer: &mut Vec<El>) -> El {
        let height = self.height.unwrap_or(theme.metrics.list_row * 1.5);
        let interactive = !self.disabled && !self.press.is_empty();
        let name = self.label.clone().or_else(|| self.tooltip.clone());
        let about = [
            self.detail.clone(),
            self.tooltip.clone().filter(|_| self.label.is_some()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ");

        let mut thumb = el()
            .key("thumb")
            .w_full()
            .h_full()
            .radius(theme.radius)
            .clip_rounded(theme.radius)
            .fill(theme.secondary.opacity(FILL).color())
            .border(BORDER, theme.border.color())
            .children(self.thumb);
        if self.dashed {
            thumb = thumb.border_dash(DASH, DASH);
        }
        if interactive && !self.selected {
            // the whole card lights the thumbnail, not only the pointer over it
            thumb = thumb.child(
                el().key("light")
                    .absolute(0., 0.)
                    .w_full()
                    .h_full()
                    .fill(theme.secondary_hover.color())
                    .group_opacity(0., 1.)
                    .a11y_hidden(),
            );
        }
        let mut frame = el()
            .key("frame")
            .w_full()
            .h(height)
            .radius(theme.radius)
            .child(thumb)
            .motion(Springs::CONTROL);
        if self.selected {
            frame = frame.outline(
                RING,
                RING_GAP,
                match self.disabled {
                    true => theme.foreground.opacity(FADED).color(),
                    false => theme.foreground.color(),
                },
            );
        }
        if self.disabled {
            frame = frame.child(
                el().absolute(0., 0.)
                    .w_full()
                    .h_full()
                    .radius(theme.radius)
                    .fill(theme.background.opacity(1. - FADED).color())
                    .a11y_hidden(),
            );
        }
        if let Some((tip, press)) = self.delete {
            let corner = theme.metrics.pad / 2.;
            let button = Button::new("delete")
                .ghost()
                .small()
                .icon("icons/trash-2.svg")
                .tooltip(tip)
                .build(theme);
            frame = frame.child(
                row()
                    .pin(None, None, Some(corner), Some(corner))
                    .child(press.apply(button, true)),
            );
        }

        let mut words = col().items_center().gap(rems(theme, 0.125)).min_w(0.);
        if self.disabled {
            words = words.opacity(FADED);
        }
        if let Some(label) = self.label.clone() {
            let (ink, weight) = match self.selected {
                true => (theme.foreground, Weight::MEDIUM),
                false => (theme.muted_foreground, Weight::NORMAL),
            };
            let lit = theme.foreground.color();
            let mut line = row().center_y().gap(rems(theme, 0.25)).min_w(0.).child(
                text(label.clone())
                    .font(TextStyle::new(theme.text(Text::Label), weight))
                    .color(ink.color())
                    .group_color(lit)
                    .truncate()
                    .shrink()
                    .min_w(0.)
                    .a11y_hidden(),
            );
            if self.variants.is_some() {
                line = line.child(
                    icon(icons::get("icons/chevron-down.svg"), CHEVRON)
                        .color(ink.color())
                        .group_color(lit)
                        .a11y_hidden(),
                );
            }
            let variants_name = i18n::t!("option-variants", name = label).into_owned();
            words = words.child(match (self.variants, self.disabled) {
                (None, _) | (Some(_), true) => line,
                (
                    Some(Variants::Menu {
                        key,
                        group,
                        access,
                        menu,
                    }),
                    false,
                ) => {
                    let layer_menu = (*menu).name(variants_name.clone());
                    Popover::new(key, &group, move |s: &mut S| access(s))
                        .face(line)
                        .name(variants_name)
                        .menu(layer_menu)
                        .placement(Placement::BottomStart)
                        .commands()
                        .build(theme, layer)
                }
                (Some(Variants::Ask(ask)), false) => row()
                    .key("variants")
                    .role("button")
                    .label(variants_name)
                    .focusable(true)
                    .cursor(Cursor::Pointer)
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| ask(s, tx))
                    .child(line),
            });
        }
        if let Some(detail) = self.detail {
            words = words.child(
                text(detail)
                    .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .truncate()
                    .a11y_hidden(),
            );
        }
        let mut card = col()
            .key(self.key)
            .role(match self.dashed {
                true => "button",
                false => "radio",
            })
            .items_center()
            .gap(rems(theme, 0.5))
            .min_w(0.)
            .hover_group_focus()
            .child(frame)
            .child(words);
        if !self.dashed {
            card = card.checked(self.selected);
        }
        if let Some(name) = name {
            card = card.label(name);
        }
        if !about.is_empty() {
            card = card.description(about);
        }
        if let Some(tip) = self.tooltip {
            card = card.help(tip);
        }
        card = match interactive {
            true => card
                .focusable(true)
                .cursor(Cursor::Pointer)
                .press_scale(PRESS_SCALE_CARD),
            false => card.disabled(self.disabled),
        };
        self.press.apply(card, interactive)
    }
}
