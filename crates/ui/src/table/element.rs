//! The track table as a Rune element: a virtualised list of rows under a sticky header with
//! sortable, resizable and movable columns, row selection, keyboard navigation, double-click to
//! open, context menus and row drags.
//!
//! The caller owns the rows and their order. [`TableState`] in its state keeps what the table
//! adds: the column layout, the sort the header shows, the selection and the keyboard cursor.
//! Rows are named by the ids the caller's `row_id` gives, so the selection follows a row through
//! a re-sort. Only the rows on screen and two viewports either side are built, so a list of
//! 100,000 rows costs what the visible thirty do.

use crate::smooth::Smooth as _;
use std::borrow::Cow;
use std::cell::Cell as Hint;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use i18n::{SharedString, t};
use rune::{
    Cursor, Drag, El, GrowthSpec, Key, KeyEvent, Mods, Phase, PointerPress, ScrollSpec, TextStyle,
    Tx, col, el, icon, row, text,
};

use super::layout::{
    ColumnSpec, Layout, PADDING, Resolved, SORT_ROOM, Sort, Sorting, TRAIL, TextAlign, Width,
    reordered, resolve, shifted, stretch,
};
use crate::color::Hsla;
use crate::glass::{bar_surface, blurring, popover_surface};
use crate::metrics::Text;
use crate::skeleton::Skeleton;
use crate::theme::Theme;

/// The narrowest a cell's content gets, however tight its column.
const MIN_CELL: f32 = 24.;
/// How wide the resize grip on a column's right edge is.
const GRIP: f32 = 9.;
/// How thick the line is that shows where a moved column lands.
const MARKER: f32 = 2.;
const SORT_ICON: f32 = 12.;
const HEAD_GAP: f32 = 4.;
/// How far a header has to be dragged before it moves its column, so a click still sorts.
const SLOP: f32 = 4.;
/// How far a row shrinks while pressed. A row spans the table, so a card's shrink would be a
/// lurch; this is just enough to feel.
const ROW_PRESS: f32 = 0.995;
/// How tall a skeleton bar stands against its row.
const SKELETON_BAR: f32 = 0.3;
/// How far a skeleton bar reaches across its cell, stepped by row and column so the column
/// edges do not line up into a grid.
const SKELETON_REACH: [f32; 5] = [0.6, 0.85, 0.45, 0.7, 0.55];
/// How many skeleton rows fill the table when the source cannot say how many are coming.
const SCREEN_ROWS: usize = 24;

type StateFn<S> = Rc<dyn Fn(&S) -> &TableState>;
type StateMutFn<S> = Rc<dyn Fn(&mut S) -> &mut TableState>;
type LenFn<S> = Rc<dyn Fn(&S) -> usize>;
type RowIdFn<S> = Rc<dyn for<'a> Fn(&'a S, usize) -> Cow<'a, str>>;
type CellFn<S, F> = Rc<dyn Fn(&S, &TableCell<F>) -> El>;
type RowFn<S, T> = Rc<dyn Fn(&S, usize) -> T>;
type FieldFn<S, F> = Rc<dyn Fn(&S, F) -> bool>;
type SortFn<S> = Rc<dyn Fn(&mut S, Option<Sorting>, &mut Tx)>;
type ChangeFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type IdFn<S> = Rc<dyn Fn(&mut S, &str, &mut Tx)>;
type IdsFn<S> = Rc<dyn Fn(&mut S, Vec<String>, &mut Tx)>;
type MenuFn<S> = Rc<dyn Fn(&mut S, Vec<String>, PointerPress, &mut Tx)>;
type HeadMenuFn<S> = Rc<dyn Fn(&mut S, PointerPress, &mut Tx)>;
type DragFn<S> = Rc<dyn Fn(&mut S, &[String], &mut Drag, &mut Tx)>;
/// Names row `i` while a handler holds the state.
type IdAt<'a> = dyn Fn(usize) -> Cow<'a, str> + 'a;

/// What one cell draws into: its column's field and inner width, the row's height, where its
/// text sits, and the row it belongs to.
pub struct TableCell<F> {
    pub field: F,
    /// The room inside the cell's padding.
    pub width: f32,
    pub height: f32,
    pub align: TextAlign,
    /// The row's index in the caller's current order.
    pub row: usize,
    pub selected: bool,
    pub playing: bool,
}

impl<F> TableCell<F> {
    /// A line of body text that ends in an ellipsis when it does not fit, in `color`.
    pub fn text(&self, content: impl Into<String>, color: Hsla, theme: &Theme) -> El {
        text(content)
            .font(TextStyle::new(theme.text(Text::Body), 400.))
            .color(color.color())
            .truncate()
            .shrink()
            .min_w(0.)
    }
}

/// The room a table holds while its source loads with nothing to show yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    /// The source knows how many rows are coming.
    Rows(usize),
    /// The count is unknown, so the skeletons fill the viewport.
    Screen,
}

/// A column the listener can show or hide, for a column menu.
pub struct Toggle {
    pub key: &'static str,
    pub label: SharedString,
    pub visible: bool,
}

/// A column resize in progress: which column, the widths when it started, and the layout widths
/// to put back if it is cancelled.
#[derive(Clone, Debug)]
struct Sizing {
    column: usize,
    widths: Vec<f32>,
    before: HashMap<String, f32>,
}

/// What the table adds to the caller's rows, kept in the caller's state: the column layout, the
/// sort, the selection and the keyboard cursor, plus a column drag in progress. Save `layout()`
/// and `sorting()` when the table reports a change; the rest is per session.
#[derive(Clone, Debug, Default)]
pub struct TableState {
    layout: Layout,
    sorting: Option<Sorting>,
    multi: bool,
    selected: HashSet<String>,
    cursor: Option<String>,
    anchor: Option<String>,
    /// Where the cursor's and the anchor's rows were last seen, to find them without a scan.
    cursor_hint: Hint<usize>,
    anchor_hint: Hint<usize>,
    sizing: Option<Sizing>,
    moving: Option<(usize, usize)>,
    /// A header was dragged since its press, so the release does not sort.
    dragged: bool,
    viewport: f32,
}

impl TableState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lets several rows be selected: Shift extends from the anchor, Cmd (Ctrl) toggles and
    /// Cmd-A selects every row.
    pub fn multi_select(mut self, on: bool) -> Self {
        self.multi = on;
        self
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
    }

    /// The sort the header shows. The caller orders its rows to match.
    pub fn sorting(&self) -> Option<&Sorting> {
        self.sorting.as_ref()
    }

    pub fn set_sorting(&mut self, sorting: Option<Sorting>) {
        self.sorting = sorting;
    }

    /// Steps `column` through its first order, the reverse, then unsorted, and returns the new
    /// sort. A column that does not sort leaves it alone.
    pub fn cycle<F>(&mut self, specs: &[ColumnSpec<F>], column: &str) -> Option<Sorting> {
        let Some(spec) = specs.iter().find(|spec| spec.key == column) else {
            return self.sorting.clone();
        };
        if !spec.sortable {
            return self.sorting.clone();
        }
        let next = match &self.sorting {
            Some(current) if current.column == column => match current.order == spec.first {
                true => Some(spec.first.reversed()),
                false => None,
            },
            _ => Some(spec.first),
        };
        self.sorting = next.map(|order| Sorting {
            column: column.to_owned(),
            order,
        });
        self.sorting.clone()
    }

    /// The order `field` is sorted in, if it is the sort column.
    pub fn direction<F: Copy + PartialEq>(
        &self,
        specs: &[ColumnSpec<F>],
        field: F,
    ) -> Option<Sort> {
        let sorting = self.sorting.as_ref()?;
        specs
            .iter()
            .find(|spec| spec.key == sorting.column)
            .filter(|spec| spec.field == field)
            .map(|_| sorting.order)
    }

    /// The columns the listener can show or hide. A column `populated` says the source has nothing
    /// for is left out, since showing it would draw nothing.
    pub fn toggles<F: Copy>(
        &self,
        specs: &[ColumnSpec<F>],
        populated: impl Fn(F) -> bool,
    ) -> Vec<Toggle> {
        specs
            .iter()
            .filter(|spec| populated(spec.field))
            .map(|spec| Toggle {
                key: spec.key,
                label: spec.label(),
                visible: !self.layout.hides(spec.key),
            })
            .collect()
    }

    /// Hides a shown column or shows a hidden one.
    pub fn toggle_column(&mut self, key: &str) {
        self.layout.toggle(key);
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selected.contains(id)
    }

    pub fn selected(&self) -> impl Iterator<Item = &str> {
        self.selected.iter().map(String::as_str)
    }

    pub fn selected_len(&self) -> usize {
        self.selected.len()
    }

    /// The keyboard cursor's row.
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Selects `id` alone and puts the cursor and the anchor on it.
    pub fn select(&mut self, id: impl Into<String>) {
        let id = id.into();
        self.selected.clear();
        self.selected.insert(id.clone());
        self.cursor = Some(id.clone());
        self.anchor = Some(id);
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
    }

    /// Selects `id`, puts the cursor on it and scrolls its row into view in the table keyed
    /// `table`.
    pub fn reveal(&mut self, table: &str, id: impl Into<String>, tx: &mut Tx) {
        let id = id.into();
        tx.scroll_into_view_in(table.to_owned(), id.clone());
        self.select(id);
    }

    /// The selected ids in the order the rows stand, or the cursor's alone when nothing is
    /// marked. `row_id` names row `i` of `len`.
    pub fn picked<'a>(&self, len: usize, row_id: impl Fn(usize) -> Cow<'a, str>) -> Vec<String> {
        if self.selected.is_empty() {
            return self.cursor.iter().cloned().collect();
        }
        if self.selected.len() == 1 {
            return self.selected.iter().cloned().collect();
        }
        (0..len)
            .map(&row_id)
            .filter(|id| self.selected.contains(id.as_ref()))
            .map(Cow::into_owned)
            .collect()
    }

    /// Forgets selected rows that are no longer among the `len` rows, after a filter or a reload.
    pub fn prune<'a>(&mut self, len: usize, row_id: impl Fn(usize) -> Cow<'a, str>) {
        if self.selected.is_empty() && self.cursor.is_none() {
            return;
        }
        let present: HashSet<String> = (0..len)
            .map(&row_id)
            .filter(|id| {
                self.selected.contains(id.as_ref())
                    || self.cursor.as_deref() == Some(id.as_ref())
                    || self.anchor.as_deref() == Some(id.as_ref())
            })
            .map(Cow::into_owned)
            .collect();
        self.selected.retain(|id| present.contains(id));
        if self.cursor.as_ref().is_some_and(|id| !present.contains(id)) {
            self.cursor = self.selected.iter().next().cloned();
        }
        if self.anchor.as_ref().is_some_and(|id| !present.contains(id)) {
            self.anchor = self.cursor.clone();
        }
    }

    fn holds(&self, id: &str) -> bool {
        self.selected.contains(id) || self.cursor.as_deref() == Some(id)
    }

    fn find(id: &str, hint: &Hint<usize>, len: usize, id_at: &IdAt<'_>) -> Option<usize> {
        let at = hint.get();
        if at < len && id_at(at) == id {
            return Some(at);
        }
        let found = (0..len).find(|&i| id_at(i) == id)?;
        hint.set(found);
        Some(found)
    }

    fn cursor_row(&self, len: usize, id_at: &IdAt<'_>) -> Option<usize> {
        Self::find(self.cursor.as_deref()?, &self.cursor_hint, len, id_at)
    }

    fn anchor_row(&self, len: usize, id_at: &IdAt<'_>) -> Option<usize> {
        Self::find(self.anchor.as_deref()?, &self.anchor_hint, len, id_at)
    }

    /// A click on row `i` with `mods`: Shift extends from the anchor, Cmd toggles the row, and a
    /// plain click selects it alone. Without multi-select every click selects one row.
    fn click(&mut self, i: usize, len: usize, mods: Mods, id_at: &IdAt<'_>) {
        let id = id_at(i).into_owned();
        if self.multi && mods.shift && self.anchor.is_some() {
            return self.extend(i, len, id_at);
        }
        self.cursor_hint.set(i);
        self.anchor_hint.set(i);
        if self.multi && mods.cmd && !mods.shift {
            match self.selected.remove(&id) {
                true => {
                    if self.cursor.as_deref() == Some(id.as_str()) {
                        self.cursor = self.selected.iter().next().cloned();
                    }
                    self.anchor = self.cursor.clone();
                }
                false => {
                    self.selected.insert(id.clone());
                    self.cursor = Some(id.clone());
                    self.anchor = Some(id);
                }
            }
            return;
        }
        self.select(id);
    }

    /// Selects every row from the anchor to row `i` and puts the cursor on `i`.
    fn extend(&mut self, i: usize, len: usize, id_at: &IdAt<'_>) {
        let anchor = self.anchor_row(len, id_at).unwrap_or_else(|| {
            self.anchor = Some(id_at(i).into_owned());
            self.anchor_hint.set(i);
            i
        });
        let (low, high) = (anchor.min(i), anchor.max(i));
        self.selected = (low..=high).map(|j| id_at(j).into_owned()).collect();
        self.cursor = Some(id_at(i).into_owned());
        self.cursor_hint.set(i);
    }

    /// Moves the cursor to row `target`, extending the selection when `extend` holds.
    fn step_to(&mut self, target: usize, len: usize, extend: bool, id_at: &IdAt<'_>) {
        match extend && self.multi {
            true => self.extend(target, len, id_at),
            false => {
                self.cursor_hint.set(target);
                self.anchor_hint.set(target);
                self.select(id_at(target).into_owned());
            }
        }
    }
}

/// A virtualised table over the caller's rows in its window state `S`, with columns described by
/// `specs`. Give it a bounded height: it is its own scroll viewport and grows to fill its parent.
/// A `lead` element (a page's hero) scrolls above the header, which then sticks to the top.
pub struct Table<S: 'static, F: Copy + PartialEq + 'static> {
    key: String,
    specs: &'static [ColumnSpec<F>],
    state: StateFn<S>,
    state_mut: StateMutFn<S>,
    len: LenFn<S>,
    row_id: RowIdFn<S>,
    width: f32,
    cell: Option<CellFn<S, F>>,
    name: Option<RowFn<S, String>>,
    playing: Option<RowFn<S, bool>>,
    tip: Option<RowFn<S, Option<String>>>,
    populated: Option<FieldFn<S, F>>,
    lead: Option<El>,
    foot: Option<El>,
    pending: Option<Pending>,
    label: Option<String>,
    bottom: f32,
    version: Option<u64>,
    on_sort: Option<SortFn<S>>,
    on_layout: Option<ChangeFn<S>>,
    on_open: Option<IdFn<S>>,
    on_select: Option<IdFn<S>>,
    on_remove: Option<IdsFn<S>>,
    on_context: Option<MenuFn<S>>,
    on_head_context: Option<HeadMenuFn<S>>,
    on_row_drag: Option<DragFn<S>>,
}

impl<S: 'static, F: Copy + PartialEq + 'static> Table<S, F> {
    /// `key` names the table's scroll viewport (scope its row ids with it). `state` and
    /// `state_mut` find the [`TableState`], `len` counts the rows, and `row_id` names row `i` in
    /// the caller's current order with an id unique among them, which keys the row.
    pub fn new(
        key: impl Into<String>,
        specs: &'static [ColumnSpec<F>],
        state: impl Fn(&S) -> &TableState + 'static,
        state_mut: impl Fn(&mut S) -> &mut TableState + 'static,
        len: impl Fn(&S) -> usize + 'static,
        row_id: impl for<'a> Fn(&'a S, usize) -> Cow<'a, str> + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            specs,
            state: Rc::new(state),
            state_mut: Rc::new(state_mut),
            len: Rc::new(len),
            row_id: Rc::new(row_id),
            width: 0.,
            cell: None,
            name: None,
            playing: None,
            tip: None,
            populated: None,
            lead: None,
            foot: None,
            pending: None,
            label: None,
            bottom: 0.,
            version: None,
            on_sort: None,
            on_layout: None,
            on_open: None,
            on_select: None,
            on_remove: None,
            on_context: None,
            on_head_context: None,
            on_row_drag: None,
        }
    }

    /// The width the columns share, from `Chrome::content` or the pane holding the table.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// What row `cell.row` shows in `cell.field`'s column, inside the cell's padding.
    pub fn cell(mut self, f: impl Fn(&S, &TableCell<F>) -> El + 'static) -> Self {
        self.cell = Some(Rc::new(f));
        self
    }

    /// What a screen reader calls row `i`, such as a track's title and artist. Without it a row
    /// is announced by its place in the list alone.
    pub fn name(mut self, f: impl Fn(&S, usize) -> String + 'static) -> Self {
        self.name = Some(Rc::new(f));
        self
    }

    /// Whether row `i` is the one playing, which tints it.
    pub fn playing(mut self, f: impl Fn(&S, usize) -> bool + 'static) -> Self {
        self.playing = Some(Rc::new(f));
        self
    }

    /// A tooltip over the whole of row `i`.
    pub fn tip(mut self, f: impl Fn(&S, usize) -> Option<String> + 'static) -> Self {
        self.tip = Some(Rc::new(f));
        self
    }

    /// Whether the source has anything for a column's field. A column without is left out.
    pub fn populated(mut self, f: impl Fn(&S, F) -> bool + 'static) -> Self {
        self.populated = Some(Rc::new(f));
        self
    }

    /// An element that scrolls above the header, such as a page's hero.
    pub fn lead(mut self, lead: El) -> Self {
        self.lead = Some(lead);
        self
    }

    /// An element that scrolls under the last row, such as a page's notices. The room set with
    /// [`Table::bottom`] then goes under it rather than under the rows.
    pub fn foot(mut self, foot: El) -> Self {
        self.foot = Some(foot);
        self
    }

    /// Skeleton rows to draw while the source has no rows yet, so the page keeps the height it is
    /// about to have.
    pub fn pending(mut self, pending: Option<Pending>) -> Self {
        self.pending = pending;
        self
    }

    /// What a screen reader calls the table.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Room under the last row, for a control perched over the table's bottom.
    pub fn bottom(mut self, room: f32) -> Self {
        self.bottom = room.max(0.);
        self
    }

    /// A number the caller changes whenever rows are reordered or replaced while their count
    /// stays the same. With it a commit skips naming every row to find out, which keeps a list
    /// of tens of thousands of rows cheap to rebuild.
    pub fn version(mut self, version: u64) -> Self {
        self.version = Some(version);
        self
    }

    /// A sortable header was clicked: the state already holds the new sort (or none); reorder
    /// the rows to match.
    pub fn on_sort(mut self, f: impl Fn(&mut S, Option<Sorting>, &mut Tx) + 'static) -> Self {
        self.on_sort = Some(Rc::new(f));
        self
    }

    /// A column was resized, moved, shown or hidden: the state's `layout()` holds the change,
    /// to be saved.
    pub fn on_layout(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.on_layout = Some(Rc::new(f));
        self
    }

    /// A row was double-clicked, or Return pressed on the cursor's row.
    pub fn on_open(mut self, f: impl Fn(&mut S, &str, &mut Tx) + 'static) -> Self {
        self.on_open = Some(Rc::new(f));
        self
    }

    /// The cursor moved to a row, by click or key.
    pub fn on_select(mut self, f: impl Fn(&mut S, &str, &mut Tx) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// Delete or Backspace was pressed with rows picked.
    pub fn on_remove(mut self, f: impl Fn(&mut S, Vec<String>, &mut Tx) + 'static) -> Self {
        self.on_remove = Some(Rc::new(f));
        self
    }

    /// A row was right-clicked: the picked rows (the clicked one alone unless it was already
    /// among them) and where, in window points, to anchor a menu.
    pub fn on_context(
        mut self,
        f: impl Fn(&mut S, Vec<String>, PointerPress, &mut Tx) + 'static,
    ) -> Self {
        self.on_context = Some(Rc::new(f));
        self
    }

    /// The header was right-clicked, where a column menu opens.
    pub fn on_head_context(mut self, f: impl Fn(&mut S, PointerPress, &mut Tx) + 'static) -> Self {
        self.on_head_context = Some(Rc::new(f));
        self
    }

    /// Every phase of a drag that starts on a row, with the rows it carries: the picked rows when
    /// it starts on one of them, else that row alone. The caller draws the ghost and the drop.
    pub fn on_row_drag(
        mut self,
        f: impl Fn(&mut S, &[String], &mut Drag, &mut Tx) + 'static,
    ) -> Self {
        self.on_row_drag = Some(Rc::new(f));
        self
    }

    pub fn build(mut self, s: &S, theme: &Theme) -> El {
        let foot = self.foot.take().map(|foot| {
            let room = std::mem::take(&mut self.bottom);
            col().key("foot").w_full().pb(room).child(foot)
        });
        let st = (self.state)(s);
        let count = (self.len)(s);
        let metrics = theme.metrics;
        let font = TextStyle::new(theme.text(Text::Body), 400.);
        let heads: Vec<f32> = self
            .specs
            .iter()
            .map(|spec| rune::text::measure(&spec.label(), &[], font, None).0)
            .collect();
        let blank: Vec<bool> = self
            .specs
            .iter()
            .map(|spec| {
                self.populated
                    .as_ref()
                    .is_some_and(|populated| !populated(s, spec.field))
            })
            .collect();
        let columns = Rc::new(resolve(
            self.specs, self.width, metrics, &st.layout, &heads, &blank,
        ));

        let header = self.header(st, &columns, theme, font);
        let body = match (count, self.pending) {
            (0, Some(pending)) => self.skeletons(pending, &columns, theme),
            _ => self.rows(count, &columns, theme),
        };

        let state_mut = self.state_mut.clone();
        let mut root = col()
            .key(self.key.clone())
            .role("grid")
            .focusable(true)
            .w_full()
            .grow(1.)
            .min_h(0.)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .smooth::<S>(&self.key)
            .on_layout_tx(move |s: &mut S, rect: [f32; 4], tx: &mut Tx| {
                let st = state_mut(s);
                if st.viewport == rect[3] {
                    tx.live_only();
                }
                st.viewport = rect[3];
            });
        if let Some(label) = &self.label {
            root = root.label(label.clone());
        }
        // Focus stays on the table while the arrow keys walk its rows, so the row they reached
        // is what a screen reader follows.
        if let Some(cursor) = &st.cursor {
            root = root.active_descendant(cursor.clone());
        }
        root = self.keys(root, metrics.row);
        if let Some(lead) = self.lead {
            root = root.child(lead);
        }
        root.child(header).child(body).children(foot)
    }

    /// The sticky header: a cell per column with its title and sort arrow, a grip on each movable
    /// column's right edge, and the marker where a moved column lands.
    fn header(
        &self,
        st: &TableState,
        columns: &Rc<Vec<Resolved<F>>>,
        theme: &Theme,
        font: TextStyle,
    ) -> El {
        let mut head = row()
            .key("head")
            .role("row")
            .w_full()
            .h(theme.metrics.header)
            .border_sides([0., 0., 1., 0.], theme.table_row_border.color())
            .sticky_top(0.);
        // With the blur on, rows pass behind the header and it frosts them: a menu's material on
        // an opaque window, the player bar's thin glass on a see-through one. Without it an
        // opaque window keeps the page under the header's own tint.
        head = match (blurring(theme), theme.transparent) {
            (true, true) => bar_surface(head, theme),
            (true, false) => popover_surface(head, theme),
            (false, true) => head.fill(theme.table_head.color()),
            (false, false) => head.fill(theme.background.blend(theme.table_head).color()),
        };
        if let Some(menu) = self.on_head_context.clone() {
            head = head.on_secondary_press(move |s: &mut S, at: PointerPress, tx: &mut Tx| {
                menu(s, at, tx)
            });
        }

        let last = columns.len().saturating_sub(1);
        let mut offset = 0.;
        let mut grips = Vec::new();
        for (ix, column) in columns.iter().enumerate() {
            head = head.child(self.head_cell(st, columns, ix, last, theme, font));
            offset += column.width;
            if ix < last && !column.spec.anchored {
                grips.push(self.grip(columns, ix, offset));
            }
        }
        head = head.children(grips);
        let marker = st.moving.and_then(|(from, to)| {
            let column = columns.get(to)?;
            let start: f32 = columns.iter().take(to).map(|column| column.width).sum();
            (from != to).then_some(match to > from {
                true => start + column.width,
                false => start,
            })
        });
        match marker {
            Some(x) => head.child(
                el().key("marker")
                    .pin(Some(x - MARKER / 2.), Some(0.), None, Some(0.))
                    .w(MARKER)
                    .fill(theme.foreground.color())
                    .a11y_hidden(),
            ),
            None => head,
        }
    }

    fn head_cell(
        &self,
        st: &TableState,
        columns: &Rc<Vec<Resolved<F>>>,
        ix: usize,
        last: usize,
        theme: &Theme,
        font: TextStyle,
    ) -> El {
        let column = &columns[ix];
        let spec = column.spec;
        let label = spec.label();
        let direction = st.direction(self.specs, spec.field);
        let dragged = st.moving.is_some_and(|(from, _)| from == ix);
        let room = match spec.sortable {
            true => SORT_ROOM,
            false => 0.,
        };
        let inner = inner_width(column.width, ix == last) - room;
        let title = text(label.to_string())
            .font(font)
            .color(match direction {
                Some(_) => theme.foreground.color(),
                None => theme.table_head_foreground.color(),
            })
            .group_color(theme.foreground.color())
            .truncate()
            .shrink()
            .min_w(0.);
        let mut cell = row()
            .key(format!("head-{}", spec.key))
            .role("columnheader")
            .label(label.to_string())
            .sorted(direction.map(|order| order == Sort::Ascending))
            .w(column.width)
            .h_full()
            .px(PADDING)
            .gap(HEAD_GAP)
            .center_y()
            .clip_soft(0.)
            .child(aligned(row().w(inner.max(0.)).center_y(), spec.align).child(title));
        if dragged {
            cell = cell.fill(theme.table_active.color());
        }
        if spec.sortable {
            let glyph = match direction {
                Some(Sort::Ascending) => "icons/chevron-up.svg",
                Some(Sort::Descending) => "icons/chevron-down.svg",
                None => "icons/chevrons-up-down.svg",
            };
            let specs = self.specs;
            let key = spec.key;
            let state_mut = self.state_mut.clone();
            let on_sort = self.on_sort.clone();
            cell = cell
                .hover_group()
                .cursor(Cursor::Pointer)
                .child(
                    icon(icons::get(glyph), SORT_ICON)
                        .color(match direction {
                            Some(_) => theme.foreground.color(),
                            None => theme.table_head_foreground.color(),
                        })
                        .group_color(theme.foreground.color()),
                )
                .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                    let st = state_mut(s);
                    if std::mem::take(&mut st.dragged) {
                        return;
                    }
                    let next = st.cycle(specs, key);
                    if let Some(on_sort) = &on_sort {
                        on_sort(s, next, tx);
                    }
                });
        }
        if !spec.anchored {
            let columns = columns.clone();
            let state_mut = self.state_mut.clone();
            let on_layout = self.on_layout.clone();
            cell = cell.on_drag(move |s: &mut S, drag: &mut Drag, tx: &mut Tx| {
                let st = state_mut(s);
                match drag.phase {
                    Phase::Down => {
                        st.dragged = false;
                        if st.moving.take().is_none() {
                            tx.live_only();
                        }
                    }
                    Phase::Move => {
                        if st.moving.is_none() && drag.dx.abs() < SLOP {
                            tx.live_only();
                            return;
                        }
                        st.dragged = true;
                        let to = shifted(&columns, ix, drag.dx);
                        if st.moving == Some((ix, to)) {
                            tx.live_only();
                            return;
                        }
                        st.moving = Some((ix, to));
                    }
                    Phase::Up => {
                        let moving = st.moving.take();
                        let Some((from, to)) = moving.filter(|_| !drag.cancelled()) else {
                            return;
                        };
                        if from == to {
                            return;
                        }
                        st.layout.order = reordered(&columns, from, to);
                        if let Some(on_layout) = &on_layout {
                            on_layout(s, tx);
                        }
                    }
                }
            });
        }
        cell
    }

    /// The drag target on column `ix`'s right edge, at `edge` points from the table's left: it
    /// widens the column and gives or takes the room from the columns after it.
    fn grip(&self, columns: &Rc<Vec<Resolved<F>>>, ix: usize, edge: f32) -> El {
        let spec = columns[ix].spec;
        let columns = columns.clone();
        let state_mut = self.state_mut.clone();
        let on_layout = self.on_layout.clone();
        el().key(format!("grip-{}", spec.key))
            .role("separator")
            .label(t!("table-resize-column", column = spec.label().to_string()).to_string())
            .pin(Some(edge - GRIP / 2.), Some(0.), None, Some(0.))
            .w(GRIP)
            .cursor(Cursor::ResizeColumn)
            .on_drag(move |s: &mut S, drag: &mut Drag, tx: &mut Tx| {
                let st = state_mut(s);
                if drag.phase == Phase::Down {
                    st.sizing = Some(Sizing {
                        column: ix,
                        widths: columns.iter().map(|column| column.width).collect(),
                        before: st.layout.widths.clone(),
                    });
                    tx.live_only();
                    return;
                }
                let Some(sizing) = st.sizing.clone().filter(|sizing| sizing.column == ix) else {
                    return;
                };
                if drag.cancelled() {
                    st.layout.widths = sizing.before;
                    st.sizing = None;
                    return;
                }
                let start: Vec<Resolved<F>> = columns
                    .iter()
                    .zip(&sizing.widths)
                    .map(|(column, width)| Resolved {
                        spec: column.spec,
                        width: *width,
                        floor: column.floor,
                    })
                    .collect();
                if let Some(widths) = stretch(&start, ix, drag.dx) {
                    st.layout.widths.extend(widths);
                }
                if drag.phase == Phase::Up {
                    st.sizing = None;
                    if let Some(on_layout) = &on_layout {
                        on_layout(s, tx);
                    }
                }
            })
    }

    /// The virtual rows: only those on screen and two viewports either side are built.
    fn rows(&self, count: usize, columns: &Rc<Vec<Resolved<F>>>, theme: &Theme) -> El {
        let state = self.state.clone();
        let state_mut = self.state_mut.clone();
        let len = self.len.clone();
        let row_id = self.row_id.clone();
        let key_id = self.row_id.clone();
        let cell = self.cell.clone();
        let name = self.name.clone();
        let playing = self.playing.clone();
        let tip = self.tip.clone();
        let on_open = self.on_open.clone();
        let on_select = self.on_select.clone();
        let on_context = self.on_context.clone();
        let on_row_drag = self.on_row_drag.clone();
        let columns = columns.clone();
        let theme = *theme;
        let height = theme.metrics.row;

        let build = move |s: &S, i: usize| -> El {
            let st = state(s);
            let id = row_id(s, i).into_owned();
            let selected = st.is_selected(&id);
            let playing = playing.as_ref().is_some_and(|playing| playing(s, i));
            let last = columns.len().saturating_sub(1);
            let cells = columns.iter().enumerate().map(|(ix, column)| {
                let spec = column.spec;
                let content = cell.as_ref().map(|cell| {
                    cell(
                        s,
                        &TableCell {
                            field: spec.field,
                            width: inner_width(column.width, ix == last),
                            height,
                            align: spec.align,
                            row: i,
                            selected,
                            playing,
                        },
                    )
                });
                let boxed = row()
                    .role("gridcell")
                    .w(column.width)
                    .h_full()
                    .pl(PADDING)
                    .pr(match ix == last {
                        true => PADDING + TRAIL,
                        false => PADDING,
                    })
                    .center_y()
                    .clip_soft(0.);
                let boxed = aligned(boxed, spec.align);
                match content {
                    Some(content) => boxed.child(content),
                    None => boxed,
                }
            });

            let mut line = row()
                .key(id.clone())
                .role("row")
                .focus_selected(selected)
                .position_in_set(i + 1, len(s))
                .w_full()
                .h(height)
                .center_y()
                .border_sides([0., 0., 1., 0.], theme.table_row_border.color())
                .press_scale(ROW_PRESS)
                .children(cells);
            if let Some(name) = &name {
                line = line.label(name(s, i));
            }
            line = match (selected, playing) {
                (true, _) => line.fill(theme.table_active.color()),
                (false, true) => line
                    .fill(theme.muted.color())
                    .hover(theme.table_hover.color()),
                (false, false) => line.hover(theme.table_hover.color()),
            };
            if let Some(tip) = tip.as_ref().and_then(|tip| tip(s, i)) {
                line = line.help(tip);
            }

            let pressed = id.clone();
            let state_mut_press = state_mut.clone();
            let row_id_press = row_id.clone();
            let len_press = len.clone();
            let on_select = on_select.clone();
            line = line.on_press_tx(move |s: &mut S, tx: &mut Tx| {
                let count = len_press(s);
                let mods = tx.mods();
                let mut st = std::mem::take(state_mut_press(s));
                let at = TableState::find(&pressed, &Hint::new(i), count, &|j| row_id_press(s, j));
                if let Some(at) = at {
                    st.click(at, count, mods, &|j| row_id_press(s, j));
                }
                *state_mut_press(s) = st;
                if let Some(on_select) = &on_select {
                    on_select(s, &pressed, tx);
                }
            });
            if let Some(on_open) = on_open.clone() {
                let opened = id.clone();
                line =
                    line.on_double_press_tx(move |s: &mut S, tx: &mut Tx| on_open(s, &opened, tx));
            }
            if let Some(on_context) = on_context.clone() {
                let clicked = id.clone();
                let state_mut = state_mut.clone();
                let row_id = row_id.clone();
                let len = len.clone();
                line = line.on_secondary_press(move |s: &mut S, at: PointerPress, tx: &mut Tx| {
                    let rows = carried(s, &clicked, &state_mut, &len, &row_id);
                    on_context(s, rows, at, tx);
                });
            }
            if let Some(on_row_drag) = on_row_drag.clone() {
                let dragged = id.clone();
                let state_mut = state_mut.clone();
                let row_id = row_id.clone();
                let len = len.clone();
                line = line.on_drag(move |s: &mut S, drag: &mut Drag, tx: &mut Tx| {
                    let rows = match state_mut(s).holds(&dragged) {
                        true => {
                            let count = len(s);
                            let st = std::mem::take(state_mut(s));
                            let rows = st.picked(count, |j| row_id(s, j));
                            *state_mut(s) = st;
                            rows
                        }
                        false => vec![dragged.clone()],
                    };
                    on_row_drag(s, &rows, drag, tx);
                });
            }
            line
        };

        let rows = col()
            .key("rows")
            .w_full()
            .growth(GrowthSpec::default())
            .pb(self.bottom)
            .virtual_rows(count, height, move |s: &S, i| key_id(s, i), build);
        match self.version {
            Some(version) => rows.virtual_rows_version(version),
            None => rows,
        }
    }

    /// Skeleton rows standing in for a source still loading its first rows, one bar per column,
    /// laid out like the rows that will replace them.
    fn skeletons(&self, pending: Pending, columns: &Rc<Vec<Resolved<F>>>, theme: &Theme) -> El {
        let count = match pending {
            Pending::Rows(count) => count,
            Pending::Screen => SCREEN_ROWS,
        };
        let height = theme.metrics.row;
        let bar = (height * SKELETON_BAR).round();
        let last = columns.len().saturating_sub(1);
        let rows = (0..count).map(|display| {
            let cells = columns.iter().enumerate().map(|(ix, column)| {
                let inner = inner_width(column.width, ix == last);
                let block = match column.spec.width {
                    Width::Thumb => Skeleton::new()
                        .build(theme)
                        .size(theme.metrics.thumb, theme.metrics.thumb),
                    _ => Skeleton::new().build(theme).size(
                        (inner * SKELETON_REACH[(display + ix) % SKELETON_REACH.len()]).round(),
                        bar,
                    ),
                };
                aligned(
                    row().w(column.width).h_full().px(PADDING).center_y(),
                    column.spec.align,
                )
                .child(block)
            });
            row()
                .key(format!("skeleton-{display}"))
                .w_full()
                .h(height)
                .center_y()
                .border_sides([0., 0., 1., 0.], theme.table_row_border.color())
                .a11y_hidden()
                .children(cells)
        });
        col().key("rows").w_full().pb(self.bottom).children(rows)
    }

    /// The table's keys: the arrows, Home, End and the page keys move the cursor (Shift extends),
    /// Return opens, Escape clears the selection, Delete removes and Cmd-A selects every row.
    fn keys(&self, root: El, row_height: f32) -> El {
        let state_mut = self.state_mut.clone();
        let len = self.len.clone();
        let row_id = self.row_id.clone();
        let table = self.key.clone();
        let on_open = self.on_open.clone();
        let on_select = self.on_select.clone();
        let on_remove = self.on_remove.clone();
        root.on_key(move |s: &mut S, event: &KeyEvent, tx: &mut Tx| {
            let count = len(s);
            let mods = event.mods;
            if count == 0 || mods.alt || (mods.ctrl && !mods.cmd) {
                return false;
            }
            let mut st = std::mem::take(state_mut(s));
            let id_at = |j: usize| row_id(s, j);
            if event.is_command('a') {
                let handled = st.multi;
                if handled {
                    st.selected = (0..count).map(|j| id_at(j).into_owned()).collect();
                }
                *state_mut(s) = st;
                return handled;
            }
            if mods.cmd {
                *state_mut(s) = st;
                return false;
            }
            let at = st.cursor_row(count, &id_at);
            let page = (st.viewport / row_height.max(1.)).floor().max(2.) as usize - 1;
            let target = match event.key {
                Key::Up => Some(at.map_or(count - 1, |at| at.saturating_sub(1))),
                Key::Down => Some(at.map_or(0, |at| (at + 1).min(count - 1))),
                Key::Home if at.is_some() => Some(0),
                Key::End if at.is_some() => Some(count - 1),
                Key::PageUp if at.is_some() => at.map(|at| at.saturating_sub(page)),
                Key::PageDown if at.is_some() => at.map(|at| (at + page).min(count - 1)),
                _ => None,
            };
            if let Some(target) = target {
                st.step_to(target, count, mods.shift, &id_at);
                let id = id_at(target).into_owned();
                *state_mut(s) = st;
                tx.scroll_into_view_in(table.clone(), id.clone());
                if let Some(on_select) = &on_select {
                    on_select(s, &id, tx);
                }
                return true;
            }
            match event.key {
                Key::Enter => {
                    let cursor = st.cursor.clone().filter(|_| st.selected.len() <= 1);
                    *state_mut(s) = st;
                    match (cursor, &on_open) {
                        (Some(id), Some(on_open)) if !tx.key_repeat() => {
                            on_open(s, &id, tx);
                            true
                        }
                        _ => false,
                    }
                }
                Key::Escape => {
                    let had = !st.selected.is_empty() || st.cursor.is_some();
                    st.clear_selection();
                    *state_mut(s) = st;
                    had
                }
                Key::Delete | Key::Backspace => {
                    let rows = st.picked(count, id_at);
                    *state_mut(s) = st;
                    match (rows.is_empty(), &on_remove) {
                        (false, Some(on_remove)) => {
                            on_remove(s, rows, tx);
                            true
                        }
                        _ => false,
                    }
                }
                _ => {
                    *state_mut(s) = st;
                    false
                }
            }
        })
    }
}

/// The rows a right-click or a drag on row `id` carries: the picked rows when `id` is among them,
/// else `id` alone, which the click also selects.
fn carried<S>(
    s: &mut S,
    id: &str,
    state_mut: &StateMutFn<S>,
    len: &LenFn<S>,
    row_id: &RowIdFn<S>,
) -> Vec<String> {
    if !state_mut(s).holds(id) {
        state_mut(s).select(id.to_owned());
        return vec![id.to_owned()];
    }
    let count = len(s);
    let st = std::mem::take(state_mut(s));
    let rows = st.picked(count, |j| row_id(s, j));
    *state_mut(s) = st;
    rows
}

/// The room inside a cell's padding, with the trailing gutter off the last column.
fn inner_width(width: f32, trailing: bool) -> f32 {
    let gutter = match trailing {
        true => TRAIL,
        false => 0.,
    };
    (width - PADDING * 2. - gutter).max(MIN_CELL)
}

fn aligned(e: El, align: TextAlign) -> El {
    match align {
        TextAlign::Left => e,
        TextAlign::Center => e.justify_center(),
        TextAlign::Right => e.justify_end(),
    }
}
