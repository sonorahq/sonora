use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Interactivity, MouseButton, Pixels,
    SharedString, Stateful, StyleRefinement, Window, div, px, relative, svg,
};

use std::time::Duration;

use i18n::t;

use crate::button::Button;
use crate::menu::Menu;
use crate::metrics::Text;
use crate::motion::{HoverFade, mix};
use crate::popover::{Popover, Popovers};
use crate::theme::ActiveTheme as _;
use crate::tooltip::{Perch, Tipped as _};

/// How thick the ring around the chosen card draws.
const RING: Pixels = px(2.);
/// The gap between the chosen card's thumbnail and its ring, left in the page colour.
const RING_GAP: Pixels = px(2.);
/// How far a card that cannot be chosen fades.
const FADED: f32 = 0.45;
/// How strongly the page shows through a card's thumbnail at rest.
const FILL: f32 = 0.45;
/// How long a card takes to brighten under the pointer, and to settle back once it leaves.
const LIGHT: Duration = Duration::from_millis(140);

/// The chevron after the name of an option that comes in variants, and the gap the menu it
/// opens leaves under the name.
const CHEVRON: Pixels = px(12.);
const MENU_GAP: Pixels = px(4.);

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// What the name of an option that comes in variants does when it is clicked.
enum Variants {
    /// Opens `menu` under the name, as the popover `key` of `group`.
    Menu {
        key: &'static str,
        group: Popovers,
        menu: Box<Menu>,
    },
    /// Runs the handler, for a card whose menu another card holds for now.
    Ask(Click),
}

/// One option of a visual choice: a thumbnail that shows what the option looks like, its name
/// under it, and a ring around the one that is chosen. Cards stand side by side in a grid, one
/// per option, and fill the column they are given. A card that can be chosen brightens its
/// edge and fill under the pointer with a short ease.
#[derive(IntoElement)]
pub struct OptionCard {
    id: ElementId,
    base: Stateful<Div>,
    thumb: Option<AnyElement>,
    label: Option<SharedString>,
    detail: Option<SharedString>,
    variants: Option<Variants>,
    height: Option<Pixels>,
    selected: bool,
    disabled: bool,
    dashed: bool,
    tooltip: Option<SharedString>,
    on_click: Option<Click>,
    /// The tooltip key and handler of the trash button in the thumbnail's corner.
    delete: Option<(SharedString, Click)>,
}

impl OptionCard {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()),
            id,
            thumb: None,
            label: None,
            detail: None,
            variants: None,
            height: None,
            selected: false,
            disabled: false,
            dashed: false,
            tooltip: None,
            on_click: None,
            delete: None,
        }
    }

    /// What the thumbnail shows. It fills the thumbnail and is cut to its corners.
    pub fn thumb(mut self, thumb: impl IntoElement) -> Self {
        self.thumb = Some(thumb.into_any_element());
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A second, smaller line under the label, such as the bitrate a quality streams at.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Puts a chevron after the name and makes the name open `menu` under it as the popover
    /// `key` of `group`, for an option that comes in variants. The name keeps its look and its
    /// line, and a click on it does not choose the card.
    pub fn variant_menu(mut self, key: &'static str, group: &Popovers, menu: Menu) -> Self {
        self.variants = Some(Variants::Menu {
            key,
            group: group.clone(),
            menu: Box::new(menu),
        });
        self
    }

    /// Puts the same chevron after the name and runs `handler` on a click on it, for a card
    /// whose variant menu another card holds while it is drawn.
    pub fn on_variants(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.variants = Some(Variants::Ask(Box::new(handler)));
        self
    }

    /// How tall the thumbnail stands, one and a half list rows unless set.
    pub fn thumb_height(mut self, height: Pixels) -> Self {
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

    /// The Fluent string `key`, shown the moment the card is hovered and following the pointer.
    /// A disabled card keeps it, so it can say why it cannot be chosen.
    pub fn tooltip(mut self, key: impl Into<SharedString>) -> Self {
        self.tooltip = Some(key.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Puts a trash button in the bottom right corner of the thumbnail, named by the tooltip
    /// `key`, for an option the user can remove. It works while the card is disabled too.
    pub fn on_delete(
        mut self,
        key: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.delete = Some((key.into(), Box::new(handler)));
        self
    }
}

impl Styled for OptionCard {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for OptionCard {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for OptionCard {}

impl RenderOnce for OptionCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            mut base,
            thumb,
            label,
            detail,
            variants,
            height,
            selected,
            disabled,
            dashed,
            tooltip,
            on_click,
            delete,
        } = self;

        let theme = *cx.theme();
        let height = height.unwrap_or(theme.metrics.list_row * 1.5);
        let interactive = !disabled && on_click.is_some();
        let reach = RING + RING_GAP;
        let overrides = std::mem::take(base.style());
        let fade = window.use_keyed_state((id.clone(), "hover"), cx, |_, _| HoverFade::new(LIGHT));
        let lit = match interactive && !selected {
            true => fade.update(cx, |fade, cx| fade.level(window, cx)),
            false => 0.,
        };
        let rest = theme.secondary.opacity(FILL);
        // The card reads as one radio of its set, or as a button when it is dashed and adds or
        // opens something. Its tooltip names a card without a label and otherwise follows the
        // detail, since it says why a disabled card cannot be chosen.
        let hint = tooltip.as_ref().map(|key| i18n::lookup(key, None));
        let name = label.clone().or_else(|| hint.clone());
        let about = [detail.clone(), hint.filter(|_| label.is_some())]
            .into_iter()
            .flatten()
            .map(|part| part.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let variants_name = label
            .as_ref()
            .map(|label| t!("option-variants", name = label.to_string()));

        let frame = div()
            .relative()
            .w_full()
            .h(height)
            .child(
                div()
                    .size_full()
                    .overflow_hidden()
                    .rounded(theme.radius)
                    .border_1()
                    .when(dashed, |this| this.border_dashed())
                    .border_color(mix(theme.border, theme.muted_foreground, lit))
                    .bg(mix(rest, theme.secondary_hover, lit))
                    .children(thumb),
            )
            // a disabled card is dimmed by a veil of the page colour rather than by opacity,
            // which would let overlapping parts of the thumb show through one another
            .when(disabled, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(theme.radius)
                        .bg(theme.background.opacity(1. - FADED)),
                )
            })
            .when(selected, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(-reach)
                        .left(-reach)
                        .right(-reach)
                        .bottom(-reach)
                        .rounded(theme.radius + reach)
                        .border_2()
                        .border_color(match disabled {
                            true => theme.foreground.opacity(FADED),
                            false => theme.foreground,
                        }),
                )
            })
            // the button keeps its press to itself so the card under it is not chosen too
            .when_some(delete, |this, (key, handler)| {
                let corner = theme.metrics.pad / 2.;
                this.child(
                    div()
                        .absolute()
                        .right(corner)
                        .bottom(corner)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new((id.clone(), "delete"))
                                .ghost()
                                .small()
                                .icon("icons/trash-2.svg")
                                .tooltip(key)
                                .on_click(move |event, window, cx| {
                                    cx.stop_propagation();
                                    handler(event, window, cx);
                                }),
                        ),
                )
            });

        let mut card = base
            .role(match dashed {
                true => gpui::accesskit::Role::Button,
                false => gpui::accesskit::Role::RadioButton,
            })
            .when_some(name, |this, name| this.aria_label(name))
            .when(!about.is_empty(), |this| this.aria_description(about))
            .when(!dashed, |this| {
                this.aria_toggled(gpui::accesskit::Toggled::from(selected))
            })
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .min_w_0()
            .when(interactive, |this| this.cursor_pointer())
            .on_hover(move |hovered, window, cx| {
                fade.update(cx, |fade, _| fade.set(*hovered));
                window.refresh();
            })
            .when_some(tooltip, |this, key| this.tip(key, Perch::Follow))
            .child(frame)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_0p5()
                    .when(disabled, |this| this.opacity(FADED))
                    .max_w_full()
                    .when_some(label, |this, label| {
                        let color = match selected {
                            true => theme.foreground,
                            false => mix(theme.muted_foreground, theme.foreground, lit),
                        };
                        let line = div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .max_w_full()
                            .text_size(theme.text(Text::Label))
                            .text_color(color)
                            .when(selected, |this| this.font_weight(FontWeight::MEDIUM))
                            .child(div().min_w_0().truncate().child(label))
                            .when(variants.is_some(), |this| {
                                this.child(
                                    svg()
                                        .path(icons::path("icons/chevron-down.svg"))
                                        .size(CHEVRON)
                                        .flex_none()
                                        .text_color(color),
                                )
                            });
                        this.child(match variants {
                            None => line.into_any_element(),
                            Some(_) if disabled => line.into_any_element(),
                            Some(Variants::Menu { key, group, menu }) => Popover::new(key, group)
                                .max_w_full()
                                .face(line)
                                .when_some(variants_name, Popover::name)
                                .menu((*menu).top(relative(1.)).mt(MENU_GAP).left_0())
                                .commands()
                                .into_any_element(),
                            Some(Variants::Ask(handler)) => div()
                                .id((id.clone(), "variants"))
                                .role(gpui::accesskit::Role::Button)
                                .when_some(variants_name, |this, name| this.aria_label(name))
                                .max_w_full()
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(move |event, window, cx| handler(event, window, cx))
                                .child(line)
                                .into_any_element(),
                        })
                    })
                    .when_some(detail, |this, detail| {
                        this.child(
                            div()
                                .max_w_full()
                                .truncate()
                                .text_size(theme.text(Text::Small))
                                .text_color(theme.muted_foreground)
                                .child(detail),
                        )
                    }),
            )
            .when(interactive, |this| {
                this.when_some(on_click, |this, handler| {
                    this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |event, window, cx| handler(event, window, cx))
                })
            });

        card.style().refine(&overrides);
        card
    }
}
