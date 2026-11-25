use std::rc::Rc;
use std::time::Duration;

use gpui::accesskit::{Role, Toggled};
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, ElementId, Hsla, PathBuilder, Pixels, SharedString, Stateful, Window,
    canvas, div, point, px, svg,
};
use i18n::t;
use ui::{ActiveTheme as _, Button, OptionCard, Perch, Room, TabBar, Tipped as _};

use super::SettingsView;

/// What a segmented control does with the option picked in it.
type Pick<T> = Rc<dyn Fn(&T, &mut Window, &mut App)>;

/// How long the dashes of a dashed outline run, how far apart they stand, and how thick its
/// line is.
const DASH: Pixels = px(6.);
const DASH_GAP: Pixels = px(4.);
const DASH_WIDTH: Pixels = px(1.);

/// How strongly a chip that is on fills with the accent, under the pointer and at rest, and
/// how strongly its border takes it.
const CHIP_FILL: f32 = 0.12;
const CHIP_HOVER: f32 = 0.2;
const CHIP_EDGE: f32 = 0.55;

/// One row of a group: the label at the start and the control at the end. A description waits
/// behind an info mark after the label, a warning behind a red mark before the control, and
/// both show their words the moment the pointer reaches them. Only a live status, as the
/// state of the update check, stays written under the label. It stands at least one list row
/// tall, with no inset, so labels line up with group headings. The label is text in the
/// accessibility tree and the marks are images named by their words, but nothing ties them to
/// the control, so the caller names a switch, picker or segmented control after the row as well.
#[derive(IntoElement)]
pub(crate) struct Row {
    title: SharedString,
    detail: Option<SharedString>,
    warning: Option<SharedString>,
    status: Option<SharedString>,
    control: AnyElement,
    disabled: bool,
    yielding: bool,
}

impl Row {
    pub(crate) fn new(title: SharedString, control: AnyElement) -> Self {
        Self {
            title,
            detail: None,
            warning: None,
            status: None,
            control,
            disabled: false,
            yielding: false,
        }
    }

    /// Lets the control give up width before the label does, for a text field that still works
    /// narrowed while a cut label says nothing.
    pub(crate) fn yielding(mut self) -> Self {
        self.yielding = true;
        self
    }

    /// Puts an explanation of the setting behind an info mark after the label.
    pub(crate) fn detail(mut self, detail: SharedString) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Puts a warning about the setting behind a red mark before the control, for a setting
    /// something else takes out of use.
    pub(crate) fn warning(mut self, warning: SharedString) -> Self {
        self.warning = Some(warning);
        self
    }

    /// Writes a live status in muted text under the label, for what the listener reads at a
    /// glance rather than looks up.
    pub(crate) fn status(mut self, status: SharedString) -> Self {
        self.status = Some(status);
        self
    }

    /// Mutes an unavailable setting's label. The caller also disables its control.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for Row {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            title,
            detail,
            warning,
            status,
            control,
            disabled,
            yielding,
        } = self;
        let theme = *cx.theme();
        let info = detail.map(|detail| {
            mark(
                SharedString::from(format!("settings-row-detail-{title}")),
                "icons/info.svg",
                theme.muted_foreground,
                detail,
                cx,
            )
        });
        let warning = warning.map(|warning| {
            mark(
                SharedString::from(format!("settings-row-warning-{title}")),
                "icons/triangle-alert.svg",
                theme.status_error,
                warning,
                cx,
            )
        });

        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .min_h(theme.metrics.list_row)
            .when(disabled, |this| this.text_color(theme.muted_foreground))
            .child(
                div()
                    .map(|this| match yielding {
                        true => this.flex_none(),
                        false => this.flex_1().min_w_0(),
                    })
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .child(
                                ui::spoken(
                                    SharedString::from(format!("settings-row-{title}")),
                                    title.clone(),
                                )
                                .min_w_0()
                                .truncate(),
                            )
                            .children(info),
                    )
                    .children(status.map(|status| {
                        div()
                            .id(SharedString::from(format!("settings-row-status-{title}")))
                            .role(Role::Paragraph)
                            .aria_label(status.clone())
                            .pr_8()
                            .text_size(theme.text(ui::Text::Small))
                            .text_color(theme.muted_foreground)
                            .child(status)
                    })),
            )
            .child(
                div()
                    .map(|this| match yielding {
                        true => this.flex_1().min_w_0().flex().justify_end(),
                        false => this.flex_none(),
                    })
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(warning)
                    .child(control),
            )
    }
}

/// A small mark of `icon` in `color` that shows `words` the moment the pointer reaches it, for
/// a description or a warning that would crowd the page written out. Screen readers hear it as
/// an image named by its words.
pub(crate) fn mark(
    id: SharedString,
    icon: &'static str,
    color: Hsla,
    words: SharedString,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();

    div()
        .id(id)
        .role(Role::Image)
        .aria_label(words.clone())
        .flex_none()
        .flex()
        .items_center()
        .child(
            svg()
                .path(icons::path(icon))
                .size(theme.text(ui::Text::Label))
                .text_color(color),
        )
        .tip_label(words, Perch::Below)
        .tooltip_show_delay(Duration::ZERO)
}

impl SettingsView {
    /// Whether Soloist plays Spotify right now, which takes some settings out of use.
    pub(super) fn soloist_plays(&self, cx: &App) -> bool {
        self.soloist.read(cx).plays(cx)
    }

    /// The note under a setting of Sonora's own sound path, which Soloist goes around. While
    /// Soloist plays Spotify such a setting still reaches local files, so it stays in use and
    /// says so. `None` while Soloist does not play.
    pub(super) fn local_only(&self, cx: &App) -> Option<SharedString> {
        self.soloist_plays(cx)
            .then(|| t!("settings-soloist-local-only"))
    }

    /// A row for a setting Soloist takes out of use: muted, with the reason behind a red mark
    /// before its control, while `unavailable`. The caller disables the control.
    pub(super) fn soloist_row_for(
        &self,
        title: SharedString,
        control: AnyElement,
        unavailable: bool,
    ) -> AnyElement {
        let row = Row::new(title, control).disabled(unavailable);
        match unavailable {
            true => row.warning(t!("settings-soloist-unavailable")),
            false => row,
        }
        .into_any_element()
    }

    /// A plain row: the label and the control.
    pub(super) fn row(&self, title: SharedString, control: AnyElement) -> AnyElement {
        Row::new(title, control).into_any_element()
    }

    /// A row whose control narrows before its label is cut, for a text field.
    pub(super) fn yielding_row(&self, title: SharedString, control: AnyElement) -> AnyElement {
        Row::new(title, control).yielding().into_any_element()
    }

    /// A row whose choice is drawn as option cards: the label on a row of its own, then the
    /// cards across the column under it.
    pub(super) fn stacked(&self, title: SharedString, cards: impl IntoElement) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .pb_3()
            .child(Row::new(title, div().into_any_element()))
            .child(cards)
            .into_any_element()
    }
}

/// What an option card says to screen readers. The card draws its name but puts nothing in the
/// accessibility tree by itself, so each settings page announces its cards through this.
pub(super) trait Announced: StatefulInteractiveElement + Sized {
    /// Announces the card as the radio button at `place` among `count` options, named `name`
    /// and checked while `chosen`.
    fn choice(self, name: SharedString, chosen: bool, place: usize, count: usize) -> Self {
        self.role(Role::RadioButton)
            .aria_label(name)
            .aria_toggled(Toggled::from(chosen))
            .aria_position_in_set(place + 1)
            .aria_size_of_set(count)
    }
}

impl Announced for OptionCard {}

/// A choice between a few options as one segmented control, each option a segment with its
/// label and the `chosen` one lit. `pick` hears the option a click lands on.
pub(super) fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: impl IntoIterator<Item = (T, SharedString)>,
    chosen: T,
    pick: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> TabBar {
    let pick: Pick<T> = Rc::new(pick);

    TabBar::new(id)
        .flex_none()
        .items(
            options
                .into_iter()
                .enumerate()
                .map(|(index, (option, label))| {
                    let pick = pick.clone();
                    Button::new((id, index))
                        .label(label)
                        .small()
                        .ghost()
                        .selected(option == chosen)
                        .on_click(move |_, window, cx| pick(&option, window, cx))
                }),
        )
}

/// One setting that turns on and off, drawn as a chip: its icon and short `label` in a bordered
/// button, washed and edged with the accent while `on` and muted while off. A few stand side by
/// side at the end of one row, so related switches share it. Screen readers hear a toggle
/// button called `name`, the setting's full name, since the short label leans on the row's.
pub(super) fn chip(
    id: impl Into<ElementId>,
    icon: &'static str,
    label: SharedString,
    name: SharedString,
    on: bool,
    cx: &App,
) -> Button {
    let theme = cx.theme();
    // a chip whose label already says it all is named by its label alone, so it is read once
    let named = name != label;

    Button::new(id)
        .outline()
        .small()
        .icon(icon)
        .label(label)
        .when(named, |chip| chip.name(name))
        .toggled(on)
        .map(|chip| match on {
            true => chip
                .fill(
                    theme.primary.opacity(CHIP_FILL),
                    theme.primary.opacity(CHIP_HOVER),
                )
                .border_color(theme.primary.opacity(CHIP_EDGE)),
            false => chip.tint(theme.muted_foreground),
        })
}

/// Chips set side by side at the end of a row.
pub(super) fn chips(chips: impl IntoIterator<Item = Button>) -> Div {
    div().flex().items_center().gap_2().children(chips)
}

/// A dashed outline over its parent's edge, rounded by `radius`, for a card standing for
/// something not set up yet. GPUI's dashed border sizes its dashes by its width, which leaves a
/// hairline dotted, so the outline is stroked here with longer dashes. The parent is
/// positioned and keeps a transparent border of the line's width, so nothing moves when the
/// outline gives way to a border.
pub(super) fn dashed(color: Hsla, radius: Pixels) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let half = DASH_WIDTH / 2.;
            let (left, top) = (bounds.left() + half, bounds.top() + half);
            let (right, bottom) = (bounds.right() - half, bounds.bottom() - half);
            if right <= left || bottom <= top {
                return;
            }
            let round = (radius - half)
                .min((right - left) / 2.)
                .min((bottom - top) / 2.)
                .max(Pixels::ZERO);
            let corner = point(round, round);
            let mut path = PathBuilder::stroke(DASH_WIDTH).dash_array(&[DASH, DASH_GAP]);
            path.move_to(point(left + round, top));
            path.line_to(point(right - round, top));
            path.arc_to(corner, Pixels::ZERO, false, true, point(right, top + round));
            path.line_to(point(right, bottom - round));
            path.arc_to(
                corner,
                Pixels::ZERO,
                false,
                true,
                point(right - round, bottom),
            );
            path.line_to(point(left + round, bottom));
            path.arc_to(
                corner,
                Pixels::ZERO,
                false,
                true,
                point(left, bottom - round),
            );
            path.line_to(point(left, top + round));
            path.arc_to(corner, Pixels::ZERO, false, true, point(left + round, top));
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    // a positioned child stands inside its parent's border, so the outline reaches back over
    // the border's width to trace the edge a border would
    .absolute()
    .top(-DASH_WIDTH)
    .right(-DASH_WIDTH)
    .bottom(-DASH_WIDTH)
    .left(-DASH_WIDTH)
}

/// A row that opens a page of its own: the label, the value chosen there in muted text, and a
/// chevron. The whole row is the button.
pub(super) fn drill(
    id: impl Into<ElementId>,
    title: SharedString,
    value: Option<SharedString>,
    cx: &App,
) -> Button {
    whole(id, title, value, "icons/chevron-right.svg", cx)
}

/// A row that opens something outside Sonora, a page in the browser or a file, marked so.
pub(super) fn outward(
    id: impl Into<ElementId>,
    title: SharedString,
    value: Option<SharedString>,
    cx: &App,
) -> Button {
    whole(id, title, value, "icons/external-link.svg", cx)
}

/// A row that is one button from edge to edge, ending in `trailing`. Its hover reaches a little
/// past the column on both sides, so the label still lines up with the rows around it.
fn whole(
    id: impl Into<ElementId>,
    title: SharedString,
    value: Option<SharedString>,
    trailing: &'static str,
    cx: &App,
) -> Button {
    let theme = cx.theme();

    Button::new(id)
        .ghost()
        .label(title)
        .when_some(value, Button::detail)
        .trailing(trailing)
        .h(theme.metrics.list_row)
        .mx_neg_3()
        .px_3()
        .gap_3()
}

/// The grid option cards stand in: `columns` across where the page is wide, fewer as it narrows.
pub(super) fn grid(columns: u16, window: &Window, cx: &App) -> Div {
    let columns = match super::Layout::of(window, cx).room() {
        Room::Tight => columns.min(2),
        Room::Snug => columns.min(3),
        Room::Roomy => columns.min(4),
        Room::Wide | Room::Vast => columns,
    };
    div().grid().grid_cols(columns.max(1)).gap_3()
}
