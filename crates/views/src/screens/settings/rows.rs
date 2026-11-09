//! The pieces every settings page is built from: a row with its label and control, a group
//! under its eyebrow, a segmented choice, the grid option cards stand in, and the rows that are
//! one button from edge to edge.

use std::rc::Rc;

use rune::{El, Transition, col, icon, row, text};
use ui::{Button, Room, TabBar, Text, Theme, Weight, font};

use crate::Ui;

/// The space between two groups of a page, and between the sections of a tab.
pub(crate) const GROUPS: f32 = 40.;
/// The space between option cards, and between the cards and the rows under them.
const CARDS_GAP: f32 = 12.;
/// The space between a row's label and its control.
const ROW_GAP: f32 = 16.;
/// The space between a row's label and its explanation.
const DETAIL_GAP: f32 = 4.;
/// How far an explanation keeps from the control beside it.
const DETAIL_END: f32 = 32.;
/// How far a whole-row button reaches past the column, so its label lines up with the rows.
const REACH: f32 = 12.;
/// The space between an eyebrow's icon and its name.
const EYEBROW_GAP: f32 = 8.;

/// What a segmented control does with the option picked in it.
type Pick<T> = Rc<dyn Fn(&mut Ui, T)>;

/// One row of a group: the label and optional explanation at the start, the control at the end.
/// It stands at least one list row tall, with no inset, so labels line up with group headings.
/// It is keyed `settings-row-{slug}`, which is what a search result scrolls to, so give each
/// setting its own slug. Name the control after the row as well, since nothing ties the label
/// to it for screen readers.
pub(crate) struct Row {
    slug: String,
    title: String,
    detail: Option<String>,
    control: El,
    disabled: bool,
    yielding: bool,
}

impl Row {
    pub(crate) fn new(slug: impl Into<String>, title: impl Into<String>, control: El) -> Self {
        Self {
            slug: slug.into(),
            title: title.into(),
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

    /// A muted explanation below the label. It wraps well short of the control.
    pub(crate) fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Mutes an unavailable setting's label. The caller also disables its control.
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(crate) fn build(self, theme: &Theme) -> El {
        let ink = match self.disabled {
            true => theme.muted_foreground,
            false => theme.foreground,
        };
        let mut words = col().key("words").gap(DETAIL_GAP).child(
            text(self.title.clone())
                .key(format!("settings-row-title-{}", self.slug))
                .role("text")
                .label(self.title.clone())
                .font(font(theme, Text::Body, Weight::NORMAL))
                .color(ink.color())
                .truncate(),
        );
        if let Some(detail) = self.detail {
            words = words.child(
                text(detail.clone())
                    .key(format!("settings-row-detail-{}", self.slug))
                    .role("paragraph")
                    .label(detail)
                    .font(font(theme, Text::Small, Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .wrap()
                    .pr(DETAIL_END),
            );
        }
        let control = row().key("control").center_y().child(self.control);
        let (words, control) = match self.yielding {
            true => (words, control.grow(1.).shrink().min_w(0.).justify_end()),
            false => (words.grow(1.).shrink().min_w(0.), control),
        };
        row()
            .key(format!("settings-row-{}", self.slug))
            .center_y()
            .justify_between()
            .gap(ROW_GAP)
            .min_h(theme.metrics.list_row)
            .w_full()
            // a row a setting brings in, such as the cache size under its switch, fades in
            .transition(Transition::opacity())
            .child(words)
            .child(control)
    }
}

/// A switch row for a setting that is on or off. `set` hears the new value inside [`crate::act`].
pub(crate) fn switch(
    theme: &Theme,
    slug: &str,
    title: impl Into<String>,
    on: bool,
    set: impl Fn(&mut Ui, bool) + 'static,
) -> El {
    let title = title.into();
    Row::new(
        slug,
        title.clone(),
        toggle(theme, slug, title, on, false, set),
    )
    .build(theme)
}

/// The switch alone, named `title`, for a row that needs more than [`switch`] gives it.
pub(crate) fn toggle(
    theme: &Theme,
    slug: &str,
    title: impl Into<String>,
    on: bool,
    disabled: bool,
    set: impl Fn(&mut Ui, bool) + 'static,
) -> El {
    ui::Switch::new(format!("settings-switch-{slug}"), on)
        .name(title)
        .disabled(disabled)
        .on_press(crate::act(move |ui: &mut Ui| set(ui, !on)))
        .build(theme)
}

/// A group of a page: its eyebrow and icon when it has a title, its option cards, then its
/// rows, all on the page itself with no surface around them and nothing between the rows.
/// `title` is a Fluent key, which also keys the heading.
pub(crate) fn group(
    theme: &Theme,
    title: Option<(&str, &str)>,
    cards: Option<El>,
    rows: impl IntoIterator<Item = El>,
) -> El {
    let key = title.map(|(key, _)| key).unwrap_or("untitled");
    let carded = cards.is_some();
    let rows: Vec<El> = rows.into_iter().collect();
    let mut group = col()
        .key(format!("settings-group-{key}"))
        .w_full()
        .transition(Transition::opacity());
    if let Some((key, glyph)) = title {
        group = group.child(heading(theme, key, Some(glyph)));
    }
    if let Some(cards) = cards {
        group = group.child(cards);
    }
    if !rows.is_empty() {
        let mut list = col().key("rows").w_full().children(rows);
        if carded {
            list = list.mt(CARDS_GAP);
        }
        group = group.child(list);
    }
    group
}

/// A group's heading: its icon before its name as an uppercase eyebrow, read as a heading of
/// the page.
pub(crate) fn heading(theme: &Theme, key: &str, glyph: Option<&str>) -> El {
    let label = i18n::lookup(key, None).into_owned();
    let size = theme.text(Text::Small);
    row()
        .key(format!("settings-heading-{key}"))
        .role("heading")
        .level(2)
        .label(label.clone())
        .center_y()
        .gap(EYEBROW_GAP)
        .mb(theme.metrics.pad)
        .children(glyph.map(|glyph| {
            icon(icons::get(glyph), size)
                .color(theme.muted_foreground.color())
                .a11y_hidden()
        }))
        .child(
            text(ui::upper(&label))
                .font(font(theme, Text::Small, Weight::SEMIBOLD))
                .color(theme.muted_foreground.color())
                .a11y_hidden(),
        )
}

/// A choice between a few options as one segmented control named `name`, each option a segment
/// with its label and the `chosen` one lit, the lit fill sliding to the next pick. `pick` hears
/// the option a press lands on, inside [`crate::act`].
pub(crate) fn segmented<T: Copy + PartialEq + 'static>(
    theme: &Theme,
    key: &str,
    name: impl Into<String>,
    options: impl IntoIterator<Item = (T, String)>,
    chosen: T,
    pick: impl Fn(&mut Ui, T) + 'static,
) -> El {
    let pick: Pick<T> = Rc::new(pick);
    let options: Vec<(T, String)> = options.into_iter().collect();
    TabBar::new(format!("settings-segments-{key}"))
        .name(name)
        .items(
            options
                .into_iter()
                .enumerate()
                .map(|(index, (option, label))| {
                    let pick = pick.clone();
                    Button::new(format!("{key}-{index}"))
                        .label(label)
                        .small()
                        .ghost()
                        .selected(option == chosen)
                        .on_press(crate::act(move |ui: &mut Ui| pick(ui, option)))
                }),
        )
        .build(theme)
}

/// A row whose choice is drawn as option cards: the label on a row of its own, then the cards
/// across the column under it.
pub(crate) fn stacked(theme: &Theme, slug: &str, title: impl Into<String>, cards: El) -> El {
    col()
        .key(format!("settings-stack-{slug}"))
        .w_full()
        .pb(CARDS_GAP)
        .child(Row::new(slug, title, col()).build(theme))
        .child(cards)
}

/// A row that opens a page of its own: the label, the value chosen there in muted text, and a
/// chevron. The whole row is the button; chain its handler and build it with [`whole`].
pub(crate) fn drill(key: &str, title: impl Into<String>, value: Option<String>) -> Button {
    edge(key, title, value, "icons/chevron-right.svg")
}

/// A row that opens something outside Sonora, a page in the browser or a file, marked so.
pub(crate) fn outward(key: &str, title: impl Into<String>, value: Option<String>) -> Button {
    edge(key, title, value, "icons/external-link.svg")
}

/// Builds a [`drill`] or [`outward`] row: as tall as a list row, its hover reaching a little
/// past the column on both sides so its label still lines up with the rows around it.
pub(crate) fn whole(button: Button, theme: &Theme) -> El {
    button
        .build(theme)
        .h(theme.metrics.list_row)
        .mx(-REACH)
        .px(REACH)
        .gap(REACH)
}

fn edge(key: &str, title: impl Into<String>, value: Option<String>, trailing: &str) -> Button {
    let mut button = Button::new(format!("settings-row-{key}"))
        .ghost()
        .label(title)
        .trailing(trailing);
    if let Some(value) = value {
        button = button.detail(value);
    }
    button
}

/// How many option cards stand across the column: `columns` where the page is wide, fewer as it
/// narrows.
pub(crate) fn columns(columns: usize, room: Room) -> usize {
    match room {
        Room::Tight => columns.min(2),
        Room::Snug => columns.min(3),
        Room::Roomy => columns.min(4),
        Room::Wide | Room::Vast => columns,
    }
    .max(1)
}

/// The grid option cards stand in, `columns` across (see [`columns`]), each card announced as
/// one of the set.
pub(crate) fn grid(key: &str, columns: usize, cards: impl IntoIterator<Item = El>) -> El {
    let cards: Vec<El> = cards.into_iter().collect();
    let count = cards.len();
    row()
        .key(format!("settings-grid-{key}"))
        .role("radiogroup")
        .w_full()
        .equal_columns(columns.max(1))
        .gaps(CARDS_GAP, CARDS_GAP)
        .children(
            cards
                .into_iter()
                .enumerate()
                .map(|(index, card)| card.position_in_set(index + 1, count)),
        )
}

/// Whether `label` or `id` holds what was typed in a picker's search, ignoring case.
pub(crate) fn matches_query(id: &str, label: &str, asked: &str) -> bool {
    asked.is_empty() || label.to_lowercase().contains(asked) || id.to_lowercase().contains(asked)
}
