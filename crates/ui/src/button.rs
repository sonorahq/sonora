//! The one button every screen uses: ghost by default, or secondary, outline, primary or
//! destructive, regular or small, with an icon, a label, a muted detail and a trailing icon.

use std::f32::consts::TAU;

use rune::{Color, Cursor, El, LoopSpec, TextStyle, Tx, el, icon, row, text};

use crate::color::Hsla;
use crate::label::Weight;
use crate::metrics::Text;
use crate::motion::{PRESS_SCALE, Springs};
use crate::paint::{Press, press_over};
use crate::theme::Theme;

/// How far a disabled button without a fill fades its ink.
const FADED: f32 = 0.55;
/// How much of a white tint a frosted ghost or outline button shows when hovered and pressed,
/// so the wash lightens the artwork under it rather than laying a themed slab over it.
const TINT_HOVER: f32 = 0.14;
const TINT_ACTIVE: f32 = 0.24;
/// The status dot's diameter and its distance from the top left corner, as parts of the pad.
const STATUS: f32 = 0.75;
const STATUS_INSET: f32 = 0.4;
/// The active-state dot under the icon, as parts of the pad.
const INDICATOR: f32 = 0.375;
const INDICATOR_INSET: f32 = 0.125;
/// How far a small button's label sits from its edge, which text set beside one lines up with.
pub const SMALL_INSET: f32 = 8.;
/// A regular button's side padding and the space between its icon and its label.
const INSET: f32 = 12.;
const GAP: f32 = 6.;
const SMALL_GAP: f32 = 4.;
const ICON: f32 = 16.;
const BORDER: f32 = 1.;
/// How long one turn of a spinning icon takes.
const TURN: f64 = 0.6;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    Ghost,
    Secondary,
    Outline,
    Primary,
    Destructive,
}

/// The colours a button paints with in its current state.
struct Palette {
    background: Option<Hsla>,
    hover: Option<Hsla>,
    active: Option<Hsla>,
    foreground: Hsla,
    border: Option<Hsla>,
}

/// A button. Build it with `.build(&theme)` and it comes back as the pressable element itself,
/// keyed, with its role and name, so a caller may chain layout onto it. Handlers go through
/// [`Button::on_press`], which a disabled button drops.
pub struct Button {
    key: String,
    label: Option<String>,
    /// Labels the button may switch between. The label slot is as wide as the widest of them,
    /// so the button keeps one width as its label changes.
    reserve: Vec<String>,
    /// What a screen reader calls the button when that differs from the visible label.
    name: Option<String>,
    icon: Option<String>,
    /// A small element drawn after the icon and before the label, such as a picker's sketch.
    leading: Option<El>,
    detail: Option<String>,
    trailing: Option<String>,
    variant: Variant,
    small: bool,
    disabled: bool,
    loading: bool,
    selected: bool,
    expanded: Option<bool>,
    toggled: Option<bool>,
    tab: bool,
    indicator: bool,
    status: Option<Hsla>,
    backgroundless: bool,
    hoverless: bool,
    frosted: bool,
    turn: f32,
    spinning: bool,
    fill: Option<(Hsla, Hsla)>,
    tint: Option<Hsla>,
    /// The tooltip text, already resolved, and whether it sits above the button.
    tooltip: Option<(String, bool)>,
    press: Press,
}

impl Button {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
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
            loading: false,
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
            spinning: false,
            fill: None,
            tint: None,
            tooltip: None,
            press: Press::default(),
        }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Keeps room for the widest of `labels` and centres the shown label in it. Pass every
    /// label the button can show, the current one included.
    pub fn reserve(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.reserve = labels.into_iter().map(Into::into).collect();
        self
    }

    /// How wide a regular, bordered button with an icon and room for the widest of `labels`
    /// draws, for a caller fitting a row of buttons into known space. No labels gives the width
    /// of an icon-only button.
    pub fn width(labels: impl IntoIterator<Item = impl AsRef<str>>, theme: &Theme) -> f32 {
        let style = TextStyle::new(theme.text(Text::Body), Weight::NORMAL);
        let widest = labels
            .into_iter()
            .map(|label| measure(label.as_ref(), style))
            .fold(0., f32::max);
        let label = match widest > 0. {
            true => GAP + widest.ceil(),
            false => 0.,
        };
        INSET * 2. + BORDER * 2. + ICON + label
    }

    /// Names the button for screen readers in place of its label, which is then read after the
    /// name. A picker passes the setting it changes, so the current choice is not its only name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// An icon before the label, spelled as call sites spell icons (`"icons/play.svg"`).
    pub fn icon(mut self, path: impl Into<String>) -> Self {
        self.icon = Some(path.into());
        self
    }

    pub fn leading(mut self, leading: El) -> Self {
        self.leading = Some(leading);
        self
    }

    /// A current value shown muted after the label and before the trailing icon, the way a row
    /// that opens a page of its own says what is chosen there.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn trailing(mut self, path: impl Into<String>) -> Self {
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

    /// Turns the icon into a spinning loader and holds presses while work the button started
    /// has not answered. Screen readers hear the button as unavailable meanwhile.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    /// Replaces the variant's fill and the colour it lifts to on hover, for a button that
    /// carries a colour of its own rather than the theme's. The press follows the hover.
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

    /// Announces the button as a tab of the tab list around it, read as chosen while `selected`.
    pub fn tab(mut self) -> Self {
        self.tab = true;
        self
    }

    /// Paints an active-state dot below the icon without changing the button's layout.
    pub fn indicator(mut self, indicator: bool) -> Self {
        self.indicator = indicator;
        self
    }

    /// Paints a dot of `color` over the top left corner of the icon, marking work under way,
    /// without changing the button's layout.
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

    /// Hovers with a white tint rather than the themed fill, for a button over artwork such as
    /// the fullscreen transport, where a themed fill reads as a slab.
    pub fn frosted(mut self) -> Self {
        self.frosted = true;
        self
    }

    /// Rotates the icon by `turn` of a full turn. A change springs there and settles with a
    /// slight overshoot.
    pub fn turned(mut self, turn: f32) -> Self {
        self.turn = turn;
        self
    }

    /// Spins the icon round while `spinning`, as for a refresh under way.
    pub fn spinning(mut self, spinning: bool) -> Self {
        self.spinning = spinning;
        self
    }

    /// Shows `text` under the button while the pointer rests on it. An icon-only button takes
    /// its accessible name from it.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some((text.into(), false));
        self
    }

    /// [`Button::tooltip`] above the button, for one along the bottom of the window.
    pub fn tooltip_above(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some((text.into(), true));
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

    pub fn build(self, theme: &Theme) -> El {
        let palette = self.palette(theme);
        let interactive = !self.disabled && !self.loading;
        let foreground = match self.disabled {
            true => palette.foreground,
            false => self.tint.unwrap_or(palette.foreground),
        };
        let ink = foreground.color();
        let (height, padding, gap) = match self.small {
            true => (theme.metrics.control_small, SMALL_INSET, SMALL_GAP),
            false => (theme.metrics.control, INSET, GAP),
        };
        let style = match self.small {
            true => TextStyle::new(theme.text(Text::Label), Weight::NORMAL),
            false => TextStyle::new(theme.text(Text::Body), Weight::NORMAL),
        };
        debug_assert!(
            self.label.is_some() || self.name.is_some() || self.tooltip.is_some(),
            "button {} has no label, name or tooltip and is silent to screen readers",
            self.key
        );
        let (accessible, description) = match self.name.clone() {
            Some(name) => (Some(name), self.label.clone().or_else(|| self.detail.clone())),
            None => (
                self.label
                    .clone()
                    .or_else(|| self.tooltip.as_ref().map(|(tip, _)| tip.clone())),
                self.detail.clone(),
            ),
        };

        let mut button = row()
            .key(self.key.clone())
            .role(match self.tab {
                true => "tab",
                false => "button",
            })
            .center()
            .gap(gap)
            .h(height)
            .px(padding)
            .radius(theme.radius)
            .motion(Springs::CONTROL);
        if let Some(name) = accessible {
            button = button.label(name);
        }
        if let Some(description) = description {
            button = button.description(description);
        }
        if self.selected || self.tab {
            button = button.focus_selected(self.selected);
        }
        if let Some(expanded) = self.expanded {
            button = button.expanded(expanded);
        }
        if let Some(toggled) = self.toggled {
            button = button.checked(toggled);
        }
        let background = match self.selected && !self.backgroundless {
            true => Some(theme.secondary_active),
            false => palette.background,
        };
        if let Some(background) = background {
            button = button.fill(background.color());
        }
        if let Some(border) = palette.border {
            button = button.border(BORDER, border.color());
        }
        if interactive && !self.hoverless {
            if let Some(hover) = palette.hover {
                button = button.hover(hover.color());
            }
        }
        if interactive && let Some(active) = palette.active {
            let hover = match self.hoverless {
                true => Color::TRANSPARENT,
                false => palette.hover.map(Hsla::color).unwrap_or(Color::TRANSPARENT),
            };
            button = button.press(press_over(hover, active.color()));
        }
        if let Some((tip, above)) = self.tooltip.clone().filter(|_| interactive) {
            button = button.help(tip);
            if above {
                button = button.help_above();
            }
        }

        let icon_path = match self.loading {
            true => Some("icons/loader-circle.svg".to_owned()),
            false => self.icon.clone(),
        };
        if let Some(path) = icon_path {
            let mut glyph = icon(icons::get(&path), ICON)
                .key("icon")
                .color(ink)
                .rotate(self.turn * TAU)
                .motion(Springs::SETTLE);
            if self.loading || self.spinning {
                glyph = glyph.looping(LoopSpec::rotation(TURN));
            }
            button = button.child(glyph);
        }
        if let Some(leading) = self.leading {
            button = button.child(leading);
        }
        if let Some(label) = self.label.clone() {
            let mut slot = text(label)
                .key("label")
                .font(style)
                .color(ink)
                .truncate()
                .a11y_hidden();
            if self.trailing.is_some() || self.detail.is_some() {
                slot = slot.grow(1.).shrink().min_w(0.);
            }
            if !self.reserve.is_empty() {
                let width = self
                    .reserve
                    .iter()
                    .map(|label| measure(label, style))
                    .fold(0., f32::max)
                    .ceil();
                slot = row().key("label-slot").w(width).justify_center().child(slot);
            }
            button = button.child(slot);
        }
        if let Some(detail) = self.detail.clone() {
            button = button.child(
                text(detail)
                    .key("detail")
                    .font(TextStyle::new(theme.text(Text::Label), Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .truncate()
                    .shrink()
                    .min_w(0.)
                    .a11y_hidden(),
            );
        }
        if let Some(path) = self.trailing.clone() {
            button = button.child(icon(icons::get(&path), ICON).key("trailing").color(ink));
        }
        if self.indicator {
            let diameter = theme.metrics.pad * INDICATOR;
            button = button.child(
                row()
                    .key("indicator")
                    .pin(
                        Some(0.),
                        None,
                        Some(0.),
                        Some(theme.metrics.pad * INDICATOR_INSET),
                    )
                    .justify_center()
                    .child(
                        el().size(diameter, diameter)
                            .radius(diameter / 2.)
                            .fill(ink),
                    )
                    .a11y_hidden(),
            );
        }
        if let Some(color) = self.status {
            let diameter = theme.metrics.pad * STATUS;
            let inset = theme.metrics.pad * STATUS_INSET;
            button = button.child(
                el().key("status")
                    .absolute(inset, inset)
                    .size(diameter, diameter)
                    .radius(diameter / 2.)
                    .fill(color.color())
                    .a11y_hidden(),
            );
        }
        let button = match interactive {
            true => button
                .focusable(true)
                .cursor(Cursor::Pointer)
                .press_scale(PRESS_SCALE),
            false => button.disabled(true),
        };
        let enabled = interactive && !self.press.is_empty();
        self.press.apply(button, enabled)
    }

    /// The colours of this button's variant, after its overrides and its disabled state.
    fn palette(&self, theme: &Theme) -> Palette {
        let subtle = |border| Palette {
            background: None,
            hover: Some(match self.frosted {
                true => theme.overlay_foreground.opacity(TINT_HOVER),
                false => theme.secondary_hover,
            }),
            active: Some(match self.frosted {
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
        let mut palette = match self.variant {
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
            Variant::Destructive => solid(theme.danger, theme.danger_hover, theme.danger_foreground),
        };
        if let Some((background, hover)) = self.fill {
            palette.background = Some(background);
            palette.hover = Some(hover);
            palette.active = Some(hover);
        }
        if self.backgroundless {
            palette.background = None;
            palette.hover = None;
            palette.active = None;
        }
        if self.disabled {
            palette.foreground = match palette.background.is_some() {
                true => theme.muted_foreground,
                false => theme.muted_foreground.opacity(FADED),
            };
            palette.background = palette.background.map(|_| theme.muted);
            palette.border = palette.border.map(|_| theme.border);
            palette.hover = None;
            palette.active = None;
        }
        palette
    }
}

/// How wide `label` sets in `style`, in points.
pub(crate) fn measure(label: &str, style: TextStyle) -> f32 {
    rune::text::measure(label, &[], style, None).0
}
