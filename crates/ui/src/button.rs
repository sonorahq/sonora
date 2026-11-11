use std::cell::RefCell;
use std::f32::consts::TAU;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Hsla, Interactivity, MouseButton, Pixels,
    SharedString, Stateful, StyleRefinement, Svg, TextStyleRefinement, Transformation, Window,
    canvas, div, point, px, radians, relative, size, svg,
};

use crate::glass::{blurring, frost};
use crate::metrics::{Text, text_width};
use crate::motion::{PRESS_SCALE, Springs, Sprung, pressing};
use crate::panel::Side;
use crate::theme::ActiveTheme as _;
use crate::tooltip::{Perch, Tipped as _};

const FADED: f32 = 0.55;
/// How much of a white tint a frosted ghost or outline button shows when hovered
/// and when pressed, so the fill lightens the content under the glass blur rather than
/// laying a themed slab over it.
const TINT_HOVER: f32 = 0.14;
const TINT_ACTIVE: f32 = 0.24;
/// The status dot's diameter and its distance from the top left corner, as parts of the pad.
const STATUS: f32 = 0.75;
const STATUS_INSET: f32 = 0.4;
/// How far a small button's label sits from its edge, which text set beside one lines up with.
pub const SMALL_INSET: Pixels = px(8.);
/// A regular button's side padding and the space between its icon and its label.
const INSET: Pixels = px(12.);
const GAP: Pixels = px(6.);
const ICON: Pixels = px(16.);
const BORDER: Pixels = px(1.);
/// How small a swapped icon starts as it pops in.
const POP_FROM: f32 = 0.6;
/// How far an icon on a belt travels between its place and where it has faded out.
const BELT_TRAVEL: Pixels = px(14.);
/// How close to the next icon the belt comes before it counts as there. A spring spends a
/// long while on its last hundredth, and a faint copy of the icon would hang beside it.
const BELT_REST: f32 = 0.01;
/// How close, in pixels and in scale, a resize and a pop come before they count as landed.
const RESIZE_REST: f32 = 0.02;
const POP_REST: f32 = 1e-3;
/// How far the drawn width may sit from the width the content asks for, borders included, and
/// still count as the content's. Further off, something else sizes the button.
const FIT_SLACK: f32 = 3.;

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

enum Variant {
    Ghost,
    Secondary,
    Outline,
    Primary,
    Destructive,
}

/// What a button remembers between frames: the width it springs on as its label changes, and
/// the icon it showed last, so a new one pops in. It sits in a cell so the button can step it
/// while it holds the theme.
struct Shape {
    width: Option<Sprung>,
    /// The width the content asked for last frame.
    natural: f32,
    /// How wide the button drew last, which tells a button its content sizes from one a column
    /// stretches.
    drawn: Option<f32>,
    icon: Option<SharedString>,
    pop: Sprung,
    /// The icon a swap replaced and how much of it is left, from 1 as it starts to shrink away.
    leaving: Option<(SharedString, Sprung)>,
    /// How many icons the belt was sent on by, one for each click not yet settled.
    sent: f32,
    /// How many of them it has moved by. The part past the whole number is how far the icon
    /// in view has left and the next one has come in.
    belt: Sprung,
    /// How far the label sits into a reserved slot, which glides as the label changes.
    inset: Option<Sprung>,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    base: Stateful<Div>,
    label: Option<SharedString>,
    /// Labels the button may switch between. The label slot is as wide as the widest of them,
    /// so the button keeps one width as its label changes.
    reserve: Vec<SharedString>,
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
    /// The colour of the status dot over the icon's top left corner, `None` for no dot.
    status: Option<Hsla>,
    backgroundless: bool,
    hoverless: bool,
    frosted: bool,
    turn: f32,
    /// Whether a swapped icon fades into the one it replaces in place, with no pop.
    blend: bool,
    /// The side the icon leaves towards on a click, for a button whose icon rides a belt.
    belt: Option<Side>,
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
        let id = id.into();
        Self {
            base: div().id(id.clone()),
            id,
            label: None,
            reserve: Vec::new(),
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
            status: None,
            backgroundless: false,
            hoverless: false,
            frosted: false,
            turn: 0.,
            blend: false,
            belt: None,
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

    /// Keeps room for the widest of `labels` and centres the shown label in it. A new label
    /// starts where the last one started and glides to the middle. Pass every label the button
    /// can show, the current one included.
    pub fn reserve(mut self, labels: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.reserve = labels.into_iter().map(Into::into).collect();
        self
    }

    /// How wide a regular, bordered button with an icon and room for the widest of `labels`
    /// draws, for a caller fitting a row of buttons into known space. No labels gives the width
    /// of an icon-only button. Labels are measured at the window's current text style.
    pub fn width(
        labels: impl IntoIterator<Item = impl Into<SharedString>>,
        window: &Window,
    ) -> Pixels {
        let widest = labels
            .into_iter()
            .map(|label| text_width(label, window))
            .max()
            .unwrap_or_default();
        let label = match widest > Pixels::ZERO {
            true => GAP + widest.ceil(),
            false => Pixels::ZERO,
        };
        INSET * 2. + BORDER * 2. + ICON + label
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

    /// Tells screen readers whether the menu or panel this button opens is showing. While it
    /// shows, the button drops its tooltip, which would otherwise sit over what it opened.
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

    /// Paints a dot of `color` over the top left corner of the icon, marking something under way,
    /// without changing the button's layout or hit area.
    pub fn status(mut self, color: Hsla) -> Self {
        self.status = Some(color);
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

    /// Swaps icons by fading one into the other where it stands, for two icons that share most
    /// of their shape. The strokes they share stay solid and only what differs fades, where the
    /// usual pop would show one icon shrinking inside the other.
    pub fn blending(mut self) -> Self {
        self.blend = true;
        self
    }

    /// Puts the icon on a belt. Each click slides it out towards `side`, shrinking and fading,
    /// while a copy comes in from the other side, and clicks in a row keep the belt running.
    pub fn belt(mut self, side: Side) -> Self {
        self.belt = Some(side);
        self
    }

    /// Rotates the icon by `turn` of a full turn, for an owner that animates it with `Spin`.
    pub fn turned(mut self, turn: f32) -> Self {
        self.turn = turn;
        self
    }

    pub fn tooltip(mut self, key: impl Into<SharedString>) -> Self {
        self.tooltip = Some((key.into(), Perch::Below));
        self
    }

    pub fn tooltip_above(mut self, key: impl Into<SharedString>) -> Self {
        self.tooltip = Some((key.into(), Perch::Above));
        self
    }

    pub fn tooltip_trailing(mut self, key: impl Into<SharedString>) -> Self {
        self.tooltip = Some((key.into(), Perch::Trailing));
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            mut base,
            label,
            reserve,
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
            status,
            backgroundless,
            hoverless,
            frosted,
            turn,
            blend,
            belt,
            hovered,
            fill,
            pressed,
            tint,
            tooltip,
            on_click,
        } = self;

        let shape = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(id.clone()), "shape".into()),
            cx,
            |_, _| {
                RefCell::new(Shape {
                    width: None,
                    natural: 0.,
                    drawn: None,
                    icon: icon.clone(),
                    pop: Sprung::at(1.),
                    leaving: None,
                    sent: 0.,
                    belt: Sprung::at(0.),
                    inset: None,
                })
            },
        );
        // A button that sizes itself to an icon, a label and a trailing icon springs to its new
        // width as the label changes. One the caller sizes, or with parts of its own, keeps to
        // plain layout.
        let fits = label.is_some()
            && reserve.is_empty()
            && leading.is_none()
            && detail.is_none()
            && sized_by_content(base.style());

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
        // The reserved slot's width, and how wide the label shown in it sets.
        let reserved = (!reserve.is_empty()).then(|| {
            let size = small.then(|| TextStyleRefinement {
                font_size: Some(theme.text(Text::Label).into()),
                ..Default::default()
            });
            window.with_text_style(size, |window| {
                let slot = reserve
                    .into_iter()
                    .map(|label| text_width(label, window))
                    .max()
                    .unwrap_or_default()
                    .ceil();
                let shown = label
                    .as_ref()
                    .map(|label| text_width(label.clone(), window))
                    .unwrap_or_default();
                (slot, shown)
            })
        });
        let (height, padding, gap) = match small {
            true => (theme.metrics.control_small, SMALL_INSET, px(4.)),
            false => (theme.metrics.control, INSET, GAP),
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
        let natural = label.as_ref().filter(|_| fits).map(|label| {
            let mut text = TextStyleRefinement::default();
            if small {
                text.font_size = Some(theme.text(Text::Label).into());
            }
            text.refine(&overrides.text);
            let label =
                window.with_text_style(Some(text), |window| text_width(label.clone(), window));
            let parts = [
                icon.as_ref().map(|_| ICON),
                Some(label.ceil()),
                trailing.as_ref().map(|_| ICON),
            ];
            let count = parts.iter().flatten().count() as f32;
            let border = match palette.border.is_some() {
                true => BORDER * 2.,
                false => Pixels::ZERO,
            };
            padding * 2.
                + border
                + parts.iter().flatten().copied().sum::<Pixels>()
                + gap * (count - 1.)
        });
        let (resizing, pop, leaving, inset, belted) = {
            let mut shape = shape.read(cx).borrow_mut();
            if natural.is_none() {
                shape.width = None;
            }
            let resizing = natural.and_then(|natural| {
                let target = f32::from(natural);
                let (last, drawn) = (shape.natural, shape.drawn);
                shape.natural = target;
                let width = shape.width.get_or_insert_with(|| Sprung::at(target));
                // A button at rest that drew wider or narrower than its content was sized by
                // its parent, so it takes the new width without springing to it.
                if width.resting() && drawn.is_some_and(|drawn| (drawn - last).abs() > FIT_SLACK) {
                    width.jump(target);
                }
                let at = width.follow(target, Springs::RESIZE, RESIZE_REST, window, cx);
                ((at - target).abs() > RESIZE_REST).then(|| px(at))
            });
            // A new icon grows in from small while the one it replaces shrinks away over it.
            if shape.icon != icon {
                if let (Some(old), Some(_)) = (shape.icon.clone(), icon.as_ref()) {
                    shape.pop.jump(0.);
                    shape.leaving = Some((old, Sprung::at(1.)));
                }
                shape.icon = icon.clone();
            }
            let pop = shape.pop.follow(1., Springs::SWAP_IN, POP_REST, window, cx);
            // A blend keeps the old icon for as long as the new one is coming in, so the two
            // always add up to solid strokes where they agree.
            let leaving = shape.leaving.as_mut().map(|(path, left)| {
                let left = match blend {
                    true => 1. - pop,
                    false => left.follow(0., Springs::SWAP_OUT, POP_REST, window, cx),
                };
                (path.clone(), left)
            });
            let leaving = leaving.filter(|(_, left)| *left > 0.);
            if leaving.is_none() {
                shape.leaving = None;
            }
            // A changed label in a reserved slot starts where the last one started and glides to
            // its own place in the middle.
            let inset = reserved.map(|(slot, shown)| {
                let target = (f32::from(slot - shown) / 2.).max(0.);
                let inset = shape.inset.get_or_insert_with(|| Sprung::at(target));
                px(inset.follow(target, Springs::RESIZE, RESIZE_REST, window, cx))
            });
            if inset.is_none() {
                shape.inset = None;
            }
            // The belt runs to where the clicks sent it. Once there the count starts over,
            // since only the part past the whole number shows.
            let belted = belt.map(|side| {
                let sent = shape.sent;
                let at = shape
                    .belt
                    .follow(sent, Springs::BELT, BELT_REST, window, cx);
                if shape.belt.resting() {
                    shape.sent = 0.;
                    shape.belt.jump(0.);
                }
                (side, at.fract())
            });
            (resizing, pop, leaving, inset, belted)
        };
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

        let button = base
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
            // Mid-resize the box is narrower or wider than what it holds, so the content keeps
            // its own width from the leading edge and the box clips it as it grows. It leaves the
            // pixel grid while it springs, so its edge glides instead of stepping a pixel at a
            // time, and only its width keeps a fraction.
            .when_some(resizing, |this, width| {
                this.w(width)
                    .h(window.pixel_snap(height))
                    .px(window.pixel_snap(padding))
                    .gap(window.pixel_snap(gap))
                    .overflow_hidden()
                    .justify_start()
                    .pixel_snap(false)
            })
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
            .when_some(
                tooltip.filter(|_| interactive && expanded != Some(true)),
                |this, (key, perch)| this.tip(key, perch),
            )
            .when_some(hovered, |this, style| this.hover(move |_| style))
            .when_some(pressed, |this, style| this.active(move |_| style))
            .when_some(icon, |this, path| {
                let tinted = |presence: f32| foreground.opacity(presence.clamp(0., 1.));
                let popped = |presence: f32| POP_FROM + (1. - POP_FROM) * presence;
                let still = Pixels::ZERO;
                match belted.filter(|(_, moved)| *moved > 0.) {
                    // The icon in view moves off as the next one comes in behind it, each
                    // shrinking towards its far end. The fade runs on the square of how much of
                    // an icon is left, so both are faint for most of the way and the two never
                    // read as a pair side by side.
                    Some((side, moved)) => {
                        let travel = match side {
                            Side::Left => -BELT_TRAVEL,
                            Side::Right => BELT_TRAVEL,
                        };
                        let left = 1. - moved;
                        let coming = glyph(
                            path.clone(),
                            tinted(moved * moved),
                            turn,
                            popped(moved),
                            travel * (moved - 1.),
                        );
                        let going = glyph(
                            path,
                            tinted(left * left),
                            turn,
                            popped(left),
                            travel * moved,
                        );
                        this.child(
                            div()
                                .relative()
                                .flex_none()
                                .size(ICON)
                                .child(coming)
                                .child(going.absolute().top_0().left_0()),
                        )
                    }
                    None => {
                        // A blend fades each icon over half the swap, the new one first, so
                        // one of the two is always whole.
                        let (shown, gone) = match blend {
                            true => (
                                glyph(path, tinted(pop * 2.), turn, 1., still),
                                leaving.map(|(old, left)| {
                                    glyph(old, tinted(left * 2.), turn, 1., still)
                                }),
                            ),
                            false => (
                                glyph(path, tinted(pop), turn, popped(pop), still),
                                leaving.map(|(old, left)| {
                                    glyph(old, tinted(left), turn, popped(left), still)
                                }),
                            ),
                        };
                        match gone {
                            Some(gone) => this.child(
                                div()
                                    .relative()
                                    .flex_none()
                                    .size(ICON)
                                    .child(shown)
                                    .child(gone.absolute().top_0().left_0()),
                            ),
                            None => this.child(shown),
                        }
                    }
                }
            })
            .children(leading)
            .when_some(label, |this, label| match reserved.zip(inset) {
                // The slot keeps the widest label's room and the label glides inside it, off the
                // pixel grid so it moves by fractions rather than whole pixels. A label wider
                // than what is left of the slot runs on into the padding for the moment it
                // starts there.
                Some(((slot, _), inset)) => this.child(
                    div().flex_none().w(slot).pixel_snap(false).child(
                        div()
                            .pixel_snap(false)
                            .ml(inset)
                            .whitespace_nowrap()
                            .child(label),
                    ),
                ),
                None => this.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .when(trailing.is_some() || detail.is_some(), |this| this.flex_1())
                        .when(resizing.is_some(), |this| this.flex_none())
                        .child(label),
                ),
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
                        .size(ICON)
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
            .when_some(status, |this, color| {
                let diameter = theme.metrics.pad * STATUS;
                this.relative().child(
                    div()
                        .absolute()
                        .top(theme.metrics.pad * STATUS_INSET)
                        .left(theme.metrics.pad * STATUS_INSET)
                        .size(diameter)
                        .rounded_full()
                        .bg(color),
                )
            })
            // Notes how wide the button drew, so the next label change can tell whether its
            // content or its parent sized it.
            .when(natural.is_some(), |this| {
                let shape = shape.clone();
                this.child(
                    canvas(
                        move |bounds, _, cx| {
                            shape.read(cx).borrow_mut().drawn = Some(f32::from(bounds.size.width));
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
            })
            .when(interactive, |this| {
                this.when_some(on_click, |this, handler| {
                    let shape = shape.clone();
                    this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |event, window, cx| {
                            if belt.is_some() {
                                shape.update(cx, |shape, cx| {
                                    shape.get_mut().sent += 1.;
                                    cx.notify();
                                });
                            }
                            handler(event, window, cx)
                        })
                })
            });

        let mut button = match interactive {
            true => pressing(button, &id, PRESS_SCALE, window, cx),
            false => button,
        };
        button.style().refine(&overrides);
        button
    }
}

/// The button's icon in `color`, turned by `turn` of a full turn, drawn at `scale` of its size
/// and moved `shift` sideways from its place.
fn glyph(path: SharedString, color: Hsla, turn: f32, scale: f32, shift: Pixels) -> Svg {
    svg()
        .path(icons::path(path))
        .size(ICON)
        .flex_none()
        .text_color(color)
        .with_transformation(
            Transformation::rotate(radians(turn * TAU))
                .with_scaling(size(scale, scale))
                .with_translation(point(shift, Pixels::ZERO)),
        )
}

/// Whether a button's caller left its width to what it holds, so the button may spring between
/// the widths its content gives it.
fn sized_by_content(overrides: &StyleRefinement) -> bool {
    overrides.size.width.is_none()
        && overrides.min_size.width.is_none()
        && overrides.max_size.width.is_none()
        && overrides.flex_grow.is_none()
        && overrides.flex_basis.is_none()
        && overrides.padding.left.is_none()
        && overrides.padding.right.is_none()
        && overrides.gap.width.is_none()
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
