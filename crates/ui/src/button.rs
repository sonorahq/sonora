use std::f32::consts::TAU;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Hsla, Interactivity, MouseButton, Pixels,
    SharedString, Stateful, StyleRefinement, Transformation, Window, div, px, radians, relative,
    svg,
};

use crate::glass::{blurring, frost};
use crate::metrics::Text;
use crate::theme::ActiveTheme as _;
use crate::tooltip::{Perch, Tipped as _};

const FADED: f32 = 0.55;
/// How much of a white tint a frosted ghost or outline button shows when hovered
/// and when pressed, so the fill lightens the content under the glass blur rather than
/// laying a themed slab over it.
const TINT_HOVER: f32 = 0.14;
const TINT_ACTIVE: f32 = 0.24;
/// How far a small button's label sits from its edge, which text set beside one lines up with.
pub const SMALL_INSET: Pixels = px(8.);

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

enum Variant {
    Ghost,
    Secondary,
    Outline,
    Primary,
    Destructive,
}

#[derive(IntoElement)]
pub struct Button {
    base: Stateful<Div>,
    label: Option<SharedString>,
    /// What a screen reader calls the button when that differs from the visible label.
    name: Option<SharedString>,
    icon: Option<SharedString>,
    /// A small element drawn after the icon and before the label, as a picker's curve sketch.
    leading: Option<AnyElement>,
    detail: Option<SharedString>,
    trailing: Option<SharedString>,
    variant: Variant,
    small: bool,
    disabled: bool,
    selected: bool,
    /// Whether the menu or panel the button opens is showing, `None` for a button that opens
    /// nothing.
    expanded: Option<bool>,
    /// Whether a button that flips a setting is on, `None` for one that does not.
    toggled: Option<bool>,
    /// Whether the button stands for one tab of a tab list.
    tab: bool,
    indicator: bool,
    backgroundless: bool,
    hoverless: bool,
    frosted: bool,
    turn: f32,
    hovered: Option<StyleRefinement>,
    fill: Option<(Hsla, Hsla)>,
    pressed: Option<StyleRefinement>,
    tint: Option<Hsla>,
    tooltip: Option<(SharedString, Perch)>,
    on_click: Option<Click>,
}

impl Button {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            label: None,
            name: None,
            icon: None,
            leading: None,
            detail: None,
            trailing: None,
            variant: Variant::Ghost,
            small: false,
            disabled: false,
            selected: false,
            expanded: None,
            toggled: None,
            tab: false,
            indicator: false,
            backgroundless: false,
            hoverless: false,
            frosted: false,
            turn: 0.,
            hovered: None,
            fill: None,
            pressed: None,
            tint: None,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Names the button for screen readers in place of its label, which is then read after the
    /// name. A picker passes the setting it changes, so the current choice is not its only name.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }

    pub fn leading(mut self, leading: impl IntoElement) -> Self {
        self.leading = Some(leading.into_any_element());
        self
    }

    /// A current value shown muted after the label and before the trailing icon, the way a row
    /// that opens a page of its own says what is chosen there.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn trailing(mut self, path: impl Into<SharedString>) -> Self {
        self.trailing = Some(path.into());
        self
    }

    /// What a button is without asking, kept for a call site that wants to say so.
    pub fn ghost(mut self) -> Self {
        self.variant = Variant::Ghost;
        self
    }

    /// A filled neutral button, for one that floats over content and would go unseen
    /// without a surface of its own.
    pub fn secondary(mut self) -> Self {
        self.variant = Variant::Secondary;
        self
    }

    pub fn outline(mut self) -> Self {
        self.variant = Variant::Outline;
        self
    }

    pub fn primary(mut self) -> Self {
        self.variant = Variant::Primary;
        self
    }

    /// Filled in the danger colour, for an action that takes something away.
    pub fn destructive(mut self) -> Self {
        self.variant = Variant::Destructive;
        self
    }

    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    /// Replaces the variant's fill and the colour it lifts to on hover, for a
    /// button that carries a colour of its own rather than the theme's. The
    /// press state follows the hover.
    pub fn fill(mut self, background: Hsla, hover: Hsla) -> Self {
        self.fill = Some((background, hover));
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Tells screen readers whether the menu or panel this button opens is showing.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Announces the button as a toggle that is on or off, for one that flips a setting such as
    /// shuffle. It changes nothing on screen.
    pub fn toggled(mut self, toggled: bool) -> Self {
        self.toggled = Some(toggled);
        self
    }

    /// Announces the button as a tab of the tab list around it, read as chosen while `selected`
    /// is set. It changes nothing on screen.
    pub fn tab(mut self) -> Self {
        self.tab = true;
        self
    }

    /// Paints an active-state dot below the icon without changing the button's layout or hit area.
    pub fn indicator(mut self, indicator: bool) -> Self {
        self.indicator = indicator;
        self
    }

    pub fn backgroundless(mut self) -> Self {
        self.backgroundless = true;
        self
    }

    pub fn hoverless(mut self) -> Self {
        self.hoverless = true;
        self
    }

    /// Blurs whatever the hover and press fills float over. Only for buttons
    /// that sit on real content, like the fullscreen transport over the ambient
    /// background. Anywhere else the backdrop is flat paint and the blur buys
    /// nothing.
    pub fn frosted(mut self) -> Self {
        self.frosted = true;
        self
    }

    /// Rotates the icon by `turn` of a full turn, for an owner that animates it with `Spin`.
    pub fn turned(mut self, turn: f32) -> Self {
        self.turn = turn;
        self
    }

    pub fn tooltip(mut self, key: impl Into<SharedString>) -> Self {
        self.tooltip = Some((key.into(), Perch::Pointer));
        self
    }

    pub fn tooltip_above(mut self, key: impl Into<SharedString>) -> Self {
        self.tooltip = Some((key.into(), Perch::Above));
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }

    fn hover(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.hovered = Some(f(self.hovered.take().unwrap_or_default()));
        self
    }
}

impl StatefulInteractiveElement for Button {
    fn active(mut self, f: impl FnOnce(StyleRefinement) -> StyleRefinement) -> Self {
        self.pressed = Some(f(self.pressed.take().unwrap_or_default()));
        self
    }
}

struct Palette {
    background: Option<Hsla>,
    hover: Option<Hsla>,
    active: Option<Hsla>,
    foreground: Hsla,
    border: Option<Hsla>,
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            label,
            name,
            icon,
            leading,
            detail,
            trailing,
            variant,
            small,
            disabled,
            selected,
            expanded,
            toggled,
            tab,
            indicator,
            backgroundless,
            hoverless,
            frosted,
            turn,
            hovered,
            fill,
            pressed,
            tint,
            tooltip,
            on_click,
        } = self;

        let theme = cx.theme();
        // A plain ghost floats over flat paint, so it hovers with the themed
        // secondary fill that stays visible on both dark and light surfaces. A
        // frosted ghost floats over real content under blur, so it hovers with
        // a white tint instead: a themed fill would read as a solid slab over
        // the blur, and a black tint would vanish into dark artwork. Only
        // frosted buttons use the tint.
        let subtle = |border| Palette {
            background: None,
            hover: Some(match frosted {
                true => theme.overlay_foreground.opacity(TINT_HOVER),
                false => theme.secondary_hover,
            }),
            active: Some(match frosted {
                true => theme.overlay_foreground.opacity(TINT_ACTIVE),
                false => theme.secondary_active,
            }),
            foreground: theme.foreground,
            border,
        };
        let solid = |background, hover, foreground| Palette {
            background: Some(background),
            hover: Some(hover),
            active: Some(hover),
            foreground,
            border: None,
        };
        let mut palette = match variant {
            Variant::Secondary => Palette {
                background: Some(theme.secondary),
                hover: Some(theme.secondary_hover),
                active: Some(theme.secondary_active),
                foreground: theme.foreground,
                border: Some(theme.border),
            },
            Variant::Ghost => subtle(None),
            Variant::Outline => subtle(Some(theme.border)),
            Variant::Primary => solid(theme.primary, theme.primary_hover, theme.primary_foreground),
            Variant::Destructive => {
                solid(theme.danger, theme.danger_hover, theme.danger_foreground)
            }
        };
        if let Some((background, hover)) = fill {
            palette.background = Some(background);
            palette.hover = Some(hover);
            palette.active = Some(hover);
        }
        if backgroundless {
            palette.background = None;
            palette.hover = None;
            palette.active = None;
        }
        if disabled {
            palette.foreground = match palette.background.is_some() {
                true => theme.muted_foreground,
                false => theme.muted_foreground.opacity(FADED),
            };
            palette.background = palette.background.map(|_| theme.muted);
            palette.border = palette.border.map(|_| theme.border);
            palette.hover = None;
            palette.active = None;
        }

        let selected_background = theme.secondary_active;
        let radius = theme.radius;
        let interactive = !disabled;
        // The white hover tint follows `frosted` whatever the setting says, since it is
        // there for a button floating over artwork; only the blur itself is optional.
        let frosting = frosted && interactive && blurring(cx);
        let foreground = match disabled {
            true => palette.foreground,
            false => tint.unwrap_or(palette.foreground),
        };
        let (height, padding, gap) = match small {
            true => (theme.metrics.control_small, SMALL_INSET, px(4.)),
            false => (theme.metrics.control, px(12.), px(6.)),
        };
        let (hover, active) = match interactive {
            true => (palette.hover, palette.active),
            false => (None, None),
        };
        let hovered = match hoverless {
            true => None,
            false => state_style(hover, hovered, frosting),
        };
        let pressed = state_style(active, pressed, frosting);
        let overrides = std::mem::take(base.style());
        // A node reaches the tree only with a role, and an icon-only button has
        // no label of its own, so its tooltip key doubles as the accessible
        // name the same way the hover text does.
        // A nameless button still takes a role, so it lands in the tree with
        // nothing to announce. Fail loudly in dev instead of going silent.
        debug_assert!(
            label.is_some() || name.is_some() || tooltip.is_some(),
            "button without label or tooltip is invisible to screen readers"
        );
        let (accessible, description) = match name {
            Some(name) => (Some(name), label.clone().or_else(|| detail.clone())),
            None => (
                label
                    .clone()
                    .or_else(|| tooltip.as_ref().map(|(key, _)| i18n::lookup(key, None))),
                detail.clone(),
            ),
        };

        let mut button = base
            .role(match tab {
                true => gpui::accesskit::Role::Tab,
                false => gpui::accesskit::Role::Button,
            })
            .when_some(accessible, |this, name| this.aria_label(name))
            .when_some(description, |this, description| {
                this.aria_description(description)
            })
            .when(selected || tab, |this| this.aria_selected(selected))
            .when_some(expanded, |this, expanded| this.aria_expanded(expanded))
            .when_some(toggled, |this, toggled| {
                this.aria_toggled(gpui::accesskit::Toggled::from(toggled))
            })
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(gap)
            .h(height)
            .px(padding)
            .rounded(radius)
            .text_color(foreground)
            .when(small, |this| this.text_size(theme.text(Text::Label)))
            .when_some(palette.background, |this, background| this.bg(background))
            .when(selected && !backgroundless, |this| {
                this.bg(selected_background)
            })
            .when_some(palette.border, |this, border| {
                this.border_1().border_color(border)
            })
            .when(interactive, |this| this.cursor_pointer())
            .when_some(tooltip.filter(|_| interactive), |this, (key, perch)| {
                this.tip(key, perch)
            })
            .when_some(hovered, |this, style| this.hover(move |_| style))
            .when_some(pressed, |this, style| this.active(move |_| style))
            .when_some(icon, |this, path| {
                this.child(
                    svg()
                        .path(icons::path(path))
                        .size(px(16.))
                        .flex_none()
                        .text_color(foreground)
                        .with_transformation(Transformation::rotate(radians(turn * TAU))),
                )
            })
            .children(leading)
            .when_some(label, |this, label| {
                this.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .when(trailing.is_some() || detail.is_some(), |this| this.flex_1())
                        .child(label),
                )
            })
            .when_some(detail, |this, detail| {
                this.child(
                    div()
                        .flex_none()
                        .max_w(relative(0.5))
                        .truncate()
                        .text_size(theme.text(Text::Label))
                        .text_color(theme.muted_foreground)
                        .child(detail),
                )
            })
            .when_some(trailing, |this, path| {
                this.child(
                    svg()
                        .path(icons::path(path))
                        .size(px(16.))
                        .flex_none()
                        .text_color(foreground),
                )
            })
            .when(indicator, |this| {
                let diameter = theme.metrics.pad * 0.375;
                this.relative().child(
                    div()
                        .absolute()
                        .left(relative(0.5))
                        .ml(-diameter / 2.)
                        .bottom(theme.metrics.pad * 0.125)
                        .size(diameter)
                        .rounded_full()
                        .bg(foreground),
                )
            })
            .when(interactive, |this| {
                this.when_some(on_click, |this, handler| {
                    this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |event, window, cx| handler(event, window, cx))
                })
            });

        button.style().refine(&overrides);
        button
    }
}

fn state_style(
    background: Option<Hsla>,
    overrides: Option<StyleRefinement>,
    frosted: bool,
) -> Option<StyleRefinement> {
    if background.is_none() && overrides.is_none() {
        return None;
    }

    let mut style = StyleRefinement::default();
    if let Some(background) = background {
        let filled = style.bg(background);
        style = match frosted {
            true => frost(filled),
            false => filled,
        };
    }
    if let Some(overrides) = overrides {
        style.refine(&overrides);
    }
    Some(style)
}
