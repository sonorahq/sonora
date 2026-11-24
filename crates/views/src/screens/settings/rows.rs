use std::rc::Rc;

use gpui::accesskit::{Role, Toggled};
use gpui::prelude::*;
use gpui::{AnyElement, App, Div, ElementId, SharedString, Window, div};
use i18n::t;
use ui::{ActiveTheme as _, Button, OptionCard, Room, TabBar};

use super::SettingsView;

/// What a segmented control does with the option picked in it.
type Pick<T> = Rc<dyn Fn(&T, &mut Window, &mut App)>;

/// How strongly a chip that is on fills with the accent, under the pointer and at rest, and
/// how strongly its border takes it.
const CHIP_FILL: f32 = 0.12;
const CHIP_HOVER: f32 = 0.2;
const CHIP_EDGE: f32 = 0.55;

/// One row of a group: the label and optional explanation at the start, the control at the end.
/// It stands at least one list row tall, with no inset, so labels line up with group headings.
/// The label and explanation are text in the accessibility tree, but nothing ties them to the
/// control, so the caller names a switch, picker or segmented control after the row as well.
#[derive(IntoElement)]
pub(crate) struct Row {
    title: SharedString,
    detail: Option<SharedString>,
    control: AnyElement,
    disabled: bool,
    yielding: bool,
}

impl Row {
    pub(crate) fn new(title: SharedString, control: AnyElement) -> Self {
        Self {
            title,
            detail: None,
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

    /// Adds a muted explanation below the label. It wraps well short of the control, so a long
    /// one never runs up against a switch or picker.
    pub(crate) fn detail(mut self, detail: SharedString) -> Self {
        self.detail = Some(detail);
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
            control,
            disabled,
            yielding,
        } = self;
        let theme = *cx.theme();

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
                        ui::spoken(
                            SharedString::from(format!("settings-row-{title}")),
                            title.clone(),
                        )
                        .truncate(),
                    )
                    .children(detail.map(|detail| {
                        div()
                            .id(SharedString::from(format!("settings-row-detail-{title}")))
                            .role(Role::Paragraph)
                            .aria_label(detail.clone())
                            .pr_8()
                            .text_size(theme.text(ui::Text::Small))
                            .text_color(theme.muted_foreground)
                            .child(detail)
                    })),
            )
            .child(
                div()
                    .map(|this| match yielding {
                        true => this.flex_1().min_w_0().flex().justify_end(),
                        false => this.flex_none(),
                    })
                    .child(control),
            )
    }
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

    /// A row for a setting Soloist takes out of use: muted, with the reason under its label,
    /// while `unavailable`. The caller disables the control.
    pub(super) fn soloist_row_for(
        &self,
        title: SharedString,
        control: AnyElement,
        unavailable: bool,
    ) -> AnyElement {
        let row = Row::new(title, control).disabled(unavailable);
        match unavailable {
            true => row.detail(t!("settings-soloist-unavailable")),
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
