mod layout;

use std::cell::Cell as StdCell;
use std::cmp::Ordering;
use std::collections::HashSet;

use gpui::prelude::*;
use gpui::{
    AbsoluteLength, AnyElement, App, Context, Corners, Div, DragMoveEvent, Empty, Entity,
    EventEmitter, FocusHandle, Focusable, Global, Interactivity, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, Point, ScrollHandle, SharedString, Stateful, StyleRefinement, TextAlign,
    Window, actions, div, point, px, svg,
};

use crate::filters::FilterChange;
use crate::menu::Menu;
use crate::metrics::{snapped, text_width};
use crate::pin::{Pin, Pinnable};
use crate::popup::Popup;
use crate::theme::ActiveTheme as _;
use crate::{Filter, SortAxis};

pub use layout::{ColumnSpec, Layout, Sort, Sorting, Width, rank};
use layout::{PADDING, Resolved, SORT_ROOM, TRAIL, reordered, resolve, shifted, stretch};

actions!(
    table,
    [SelectNext, SelectPrevious, Deselect, Activate, Remove]
);

pub const TABLE_CONTEXT: &str = "Table";

const MIN_CELL: Pixels = px(24.);
const GRIP: Pixels = px(9.);
const OVERSCAN: usize = 2;

pub const ROW_GROUP: &str = "table-row";

pub struct Cell<F> {
    pub field: F,
    pub width: Pixels,
    pub height: Pixels,
    pub align: TextAlign,
    pub display: usize,
    pub row: usize,
}

impl<F> Cell<F> {
    pub fn frame(&self) -> Div {
        self.box_of(div())
            .min_w_0()
            .truncate()
            .text_align(self.align)
            .line_height(self.height)
    }

    pub fn middle(&self) -> Div {
        self.box_of(div()).flex().items_center()
    }

    fn box_of(&self, base: Div) -> Div {
        base.w(self.width + PADDING * 2.)
            .flex_none()
            .h_full()
            .px(PADDING)
    }
}

pub trait TableSource: 'static {
    type Field: Copy + PartialEq + 'static;

    fn columns(&self) -> &'static [ColumnSpec<Self::Field>];
    fn rows(&self, cx: &App) -> usize;

    fn populated(&self, _field: Self::Field, _cx: &App) -> bool {
        true
    }
    fn cell(&self, cell: Cell<Self::Field>, cx: &mut App) -> AnyElement;

    fn picking(&self) -> bool {
        false
    }

    fn context_menu(&self, _rows: &[usize], _visible: &[Self::Field], _cx: &App) -> Option<Menu> {
        None
    }

    fn context_menu_will_open(&self, _rows: &[usize], _cx: &App) {}

    fn pin(&self, _row: usize, _cx: &App) -> Option<Pin> {
        None
    }

    fn group(&self, _field: Self::Field, _row: usize, _cx: &App) -> Option<SharedString> {
        None
    }

    fn compare(&self, _field: Self::Field, a: usize, b: usize, _cx: &App) -> Ordering {
        a.cmp(&b)
    }

    fn matches(&self, _row: usize, _query: &str, _cx: &App) -> bool {
        true
    }

    fn filter_axes(&self, _query: &str, _cx: &App) -> Vec<Filter> {
        vec![]
    }

    /// Returns `true` if the change was applied or `false` if it's not handled by this source
    fn filter(&mut self, _change: FilterChange, _cx: &App) -> bool {
        false
    }

    fn filtered(&self, _cx: &App) -> bool {
        false
    }

    fn playing(&self, _row: usize, _cx: &App) -> bool {
        false
    }

    fn is_loading(&self, _cx: &App) -> bool {
        false
    }
}

fn frame(width: Pixels, align: TextAlign) -> Div {
    div()
        .w(width)
        .flex_none()
        .min_w_0()
        .truncate()
        .text_align(align)
}

pub struct TableDelegate<S: TableSource> {
    source: S,
    columns: Vec<Resolved<S::Field>>,
    width: Pixels,
    layout: Layout,
    heads: Vec<Pixels>,
    measured: Option<(i18n::Language, Pixels, SharedString)>,
    selected: Option<usize>,
    anchor: Option<usize>,
    marked: HashSet<usize>,
    sort: Option<(S::Field, Sort)>,
    query: String,
    order: Vec<usize>,
}

impl<S: TableSource> TableDelegate<S> {
    pub fn new(source: S, width: Pixels, cx: &App) -> Self {
        let heads = vec![Pixels::ZERO; source.columns().len()];
        let blank = vec![false; heads.len()];
        let columns = resolve(
            source.columns(),
            width,
            cx.theme().metrics,
            &Layout::default(),
            &heads,
            &blank,
        );
        let mut delegate = Self {
            source,
            columns,
            width,
            layout: Layout::default(),
            heads,
            measured: None,
            selected: None,
            anchor: None,
            marked: HashSet::new(),
            sort: None,
            query: String::new(),
            order: Vec::new(),
        };
        delegate.reorder(cx);
        delegate
    }

    fn measure(&mut self, window: &Window, cx: &App) {
        let stamp = (
            i18n::language(),
            window.rem_size(),
            window.text_style().font_family,
        );
        if self.measured.as_ref() == Some(&stamp) {
            return;
        }
        self.measured = Some(stamp);

        let heads: Vec<Pixels> = self
            .source
            .columns()
            .iter()
            .map(|spec| text_width(spec.label(), window))
            .collect();
        if heads == self.heads {
            return;
        }

        self.heads = heads;
        self.relayout(cx);
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    pub fn with_sort(mut self, field: S::Field, direction: Sort, cx: &App) -> Self {
        self.sort = Some((field, direction));
        self.reorder(cx);
        self
    }

    pub fn set_width(&mut self, width: Pixels, cx: &App) {
        self.width = width;
        self.relayout(cx);
    }

    fn rebuild(&mut self, cx: &App) {
        self.relayout(cx);
        self.reorder(cx);
    }

    pub fn row(&self, display: usize) -> usize {
        self.order.get(display).copied().unwrap_or(display)
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn picked(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self.marked.iter().copied().collect();
        if rows.is_empty() {
            return self.selected.into_iter().collect();
        }
        rows.sort_by_key(|row| self.display_of(*row).unwrap_or(usize::MAX));
        rows
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.anchor = None;
        self.marked.clear();
    }

    fn pick(&mut self, row: usize, extend: bool, toggle: bool) {
        if !self.source.picking() {
            self.marked.clear();
            self.marked.insert(row);
            self.selected = Some(row);
            self.anchor = Some(row);
            return;
        }
        if toggle {
            if self.marked.contains(&row) {
                self.marked.remove(&row);
                if self.selected == Some(row) {
                    self.selected = self.marked.iter().copied().next();
                }
                if self.anchor == Some(row) {
                    self.anchor = self.selected;
                }
            } else {
                self.marked.insert(row);
                self.selected = Some(row);
                self.anchor = Some(row);
            }
            return;
        }
        if extend && self.anchor.is_some() {
            self.selected = Some(row);
            self.marked = self.span_rows();
            return;
        }
        self.marked.clear();
        self.marked.insert(row);
        self.selected = Some(row);
        self.anchor = Some(row);
    }

    fn holds(&self, row: usize) -> bool {
        self.marked.contains(&row) || self.selected == Some(row)
    }

    fn span_rows(&self) -> HashSet<usize> {
        match self.picked_span() {
            Some((lo, hi)) => self.order[lo..=hi].iter().copied().collect(),
            None => self.selected.into_iter().collect(),
        }
    }

    fn picked_span(&self) -> Option<(usize, usize)> {
        let caret = self.display_of(self.selected?)?;
        let origin = self
            .anchor
            .and_then(|row| self.display_of(row))
            .unwrap_or(caret);
        Some(match origin <= caret {
            true => (origin, caret),
            false => (caret, origin),
        })
    }

    fn display_of(&self, row: usize) -> Option<usize> {
        self.order.iter().position(|&candidate| candidate == row)
    }

    pub fn group(&self, display: usize, cx: &App) -> Option<SharedString> {
        let (field, _) = self.sort?;
        self.source.group(field, self.row(display), cx)
    }

    pub fn row_count(&self) -> usize {
        self.order.len()
    }

    fn visible(&self) -> Vec<S::Field> {
        self.columns
            .iter()
            .map(|column| column.spec.field)
            .collect()
    }

    fn relayout(&mut self, cx: &App) {
        let blank: Vec<bool> = self
            .source
            .columns()
            .iter()
            .map(|spec| !self.source.populated(spec.field, cx))
            .collect();

        self.columns = resolve(
            self.source.columns(),
            self.width,
            cx.theme().metrics,
            &self.layout,
            &self.heads,
            &blank,
        );
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn set_layout(&mut self, layout: Layout, cx: &App) {
        self.layout = layout;
        self.relayout(cx);
    }

    pub fn sorting(&self) -> Option<Sorting> {
        let (field, order) = self.sort?;
        let column = self
            .source
            .columns()
            .iter()
            .find(|spec| spec.field == field)?;

        Some(Sorting {
            column: column.key.to_owned(),
            order,
        })
    }

    pub fn set_sorting(&mut self, sorting: Option<Sorting>, cx: &App) {
        self.sort = sorting.and_then(|sorting| {
            let column = self
                .source
                .columns()
                .iter()
                .filter(|spec| spec.sortable)
                .find(|spec| spec.key == sorting.column)?;
            Some((column.field, sorting.order))
        });
        self.reorder(cx);
    }

    pub fn toggles(&self) -> Vec<Toggle> {
        self.source
            .columns()
            .iter()
            .map(|spec| Toggle {
                key: spec.key,
                label: spec.label(),
                visible: !self.layout.hides(spec.key),
            })
            .collect()
    }

    pub fn cycle(&mut self, column: &str, cx: &App) {
        let next = match self.sorting() {
            Some(current) if current.column == column => match current.order {
                Sort::Ascending => Some(Sort::Descending),
                Sort::Descending => None,
            },
            _ => Some(Sort::Ascending),
        };
        self.set_sorting(
            next.map(|order| Sorting {
                column: column.to_owned(),
                order,
            }),
            cx,
        );
    }

    pub fn sortables(&self) -> Vec<SortAxis> {
        let active = self.sorting();
        self.source
            .columns()
            .iter()
            .filter(|spec| spec.sortable)
            .map(|spec| SortAxis {
                key: spec.key,
                label: spec.label(),
                order: active
                    .as_ref()
                    .filter(|it| it.column == spec.key)
                    .map(|it| it.order),
            })
            .collect()
    }

    pub fn set_query(&mut self, query: &str, cx: &App) {
        self.query = query.trim().to_lowercase();
        self.reorder(cx);
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    fn reorder(&mut self, cx: &App) {
        let mut order: Vec<usize> = (0..self.source.rows(cx))
            .filter(|row| {
                (self.query.is_empty() && !self.source.filtered(cx))
                    || self.source.matches(*row, &self.query, cx)
            })
            .collect();

        if let Some((field, direction)) = self.sort {
            match direction {
                Sort::Ascending => order.sort_by(|&a, &b| self.source.compare(field, a, b, cx)),
                Sort::Descending => order.sort_by(|&a, &b| self.source.compare(field, b, a, cx)),
            }
        }

        self.order = order;
        self.prune_selection();
    }

    fn prune_selection(&mut self) {
        let visible: HashSet<usize> = self
            .marked
            .iter()
            .copied()
            .filter(|row| self.display_of(*row).is_some())
            .collect();
        self.marked = visible;
        if self.selected.is_some_and(|row| !self.marked.contains(&row)) {
            self.selected = self.marked.iter().copied().next();
        }
        if self.anchor.is_some_and(|row| !self.marked.contains(&row)) {
            self.anchor = self.selected;
        }
    }

    fn inner_width(&self, col_ix: usize) -> Pixels {
        let trailing = col_ix + 1 == self.columns.len();
        let gutter = if trailing { TRAIL } else { Pixels::ZERO };
        (self.columns[col_ix].width - PADDING * 2. - gutter).max(MIN_CELL)
    }

    fn offset(&self, col_ix: usize) -> Pixels {
        self.columns
            .iter()
            .take(col_ix)
            .map(|column| column.width)
            .fold(Pixels::ZERO, |total, width| total + width)
    }

    fn direction(&self, field: S::Field) -> Option<Sort> {
        self.sort
            .filter(|(sorted, _)| *sorted == field)
            .map(|(_, direction)| direction)
    }
}

pub enum TableEvent {
    DoubleClicked(usize),
    Activated(usize),
    Removed,
    LayoutChanged,
    SortChanged,
}

pub struct Toggle {
    pub key: &'static str,
    pub label: SharedString,
    pub visible: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Viewport {
    pub top: Pixels,
    pub height: Pixels,
}

impl Viewport {
    pub fn measured(top: Pixels, height: Pixels, window: &Window) -> Self {
        Self {
            top: top.max(Pixels::ZERO),
            height: match height > Pixels::ZERO {
                true => height,
                false => window.viewport_size().height,
            },
        }
    }

    fn rows(&self, row: Pixels) -> usize {
        (self.height / row).ceil().max(0.) as usize + OVERSCAN
    }

    fn first(&self, head: Pixels, row: Pixels) -> usize {
        ((self.top - head) / row).floor().max(0.) as usize
    }
}

#[derive(Clone)]
struct ColumnResize {
    column: usize,
    origin: StdCell<Pixels>,
}

#[derive(Clone)]
struct ColumnMove {
    column: usize,
    origin: StdCell<Pixels>,
}

struct Sizing {
    column: usize,
    widths: Vec<Pixels>,
}

pub struct TableState<S: TableSource> {
    delegate: TableDelegate<S>,
    viewport: Viewport,
    corners: Corners<Pixels>,
    focus: FocusHandle,
    scroll: Option<ScrollHandle>,
    context_menu: Option<(Vec<usize>, Point<Pixels>)>,
    moving: Option<(usize, usize)>,
    sizing: Option<Sizing>,
}

impl<S: TableSource> EventEmitter<TableEvent> for TableState<S> {}

impl<S: TableSource> Focusable for TableState<S> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<S: TableSource> TableState<S> {
    pub fn new(delegate: TableDelegate<S>, cx: &mut Context<Self>) -> Self {
        Self {
            delegate,
            viewport: Viewport::default(),
            corners: Corners::default(),
            focus: cx.focus_handle(),
            scroll: None,
            context_menu: None,
            moving: None,
            sizing: None,
        }
    }

    pub fn follow(mut self, scroll: ScrollHandle) -> Self {
        self.scroll = Some(scroll);
        self
    }

    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
    }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }

    fn select_previous(&mut self, _: &SelectPrevious, window: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, window, cx);
    }

    fn deselect(&mut self, _: &Deselect, _: &mut Window, cx: &mut Context<Self>) {
        if self.context_menu.take().is_some() {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.delegate.selected.is_none() && self.delegate.marked.is_empty() {
            return;
        }
        self.delegate.clear_selection();
        cx.notify();
    }

    fn activate(&mut self, _: &Activate, _: &mut Window, cx: &mut Context<Self>) {
        self.fire_activate(cx);
    }

    fn remove(&mut self, _: &Remove, _: &mut Window, cx: &mut Context<Self>) {
        self.fire_remove(cx);
    }

    fn fire_activate(&mut self, cx: &mut Context<Self>) {
        if self.delegate.picked().len() != 1 {
            return;
        }
        let Some(row) = self.delegate.selected else {
            return;
        };
        let Some(display) = self.delegate.display_of(row) else {
            return;
        };
        cx.emit(TableEvent::Activated(display));
    }

    fn fire_remove(&mut self, cx: &mut Context<Self>) {
        if self.delegate.picked().is_empty() {
            return;
        }
        cx.emit(TableEvent::Removed);
    }

    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.delegate.row_count();
        if count == 0 {
            return;
        }

        let display = match self
            .delegate
            .selected
            .and_then(|row| self.delegate.display_of(row))
        {
            Some(current) => current.saturating_add_signed(delta).min(count - 1),
            None if delta < 0 => count - 1,
            None => 0,
        };

        self.delegate
            .pick(self.delegate.row(display), window.modifiers().shift, false);
        self.reveal(display, window, cx);
        cx.notify();
    }

    fn reveal(&self, display: usize, window: &Window, cx: &App) {
        let Some(scroll) = &self.scroll else {
            return;
        };

        let metrics = cx.theme().metrics;
        let row = snapped(metrics.row, window);
        let head = snapped(metrics.header, window);
        let top = head + row * display as f32;
        let above = self.viewport.top + head;
        let below = self.viewport.top + self.viewport.height;

        let delta = if top < above {
            top - above
        } else if top + row > below {
            top + row - below
        } else {
            return;
        };

        let offset = scroll.offset();
        scroll.set_offset(point(offset.x, offset.y - delta));
    }

    fn height(&self, head: Pixels, row: Pixels) -> Pixels {
        head + row * self.delegate.row_count() as f32
    }

    pub fn delegate(&self) -> &TableDelegate<S> {
        &self.delegate
    }

    pub fn delegate_mut(&mut self) -> &mut TableDelegate<S> {
        &mut self.delegate
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        self.delegate.rebuild(cx);
        cx.notify();
    }

    fn toggle_sort(&mut self, col_ix: usize, cx: &mut Context<Self>) {
        let Some(column) = self.delegate.columns.get(col_ix) else {
            return;
        };
        if !column.spec.sortable {
            return;
        }

        let key = column.spec.key;
        self.delegate.cycle(key, cx);
        self.delegate.rebuild(cx);
        cx.emit(TableEvent::SortChanged);
        cx.notify();
    }

    fn resize(&mut self, resize: &ColumnResize, to: Pixels, cx: &mut Context<Self>) {
        let start = match &self.sizing {
            Some(sizing) if sizing.column == resize.column => sizing.widths.clone(),
            _ => {
                let widths: Vec<Pixels> = self.delegate.columns.iter().map(|it| it.width).collect();
                self.sizing = Some(Sizing {
                    column: resize.column,
                    widths: widths.clone(),
                });
                widths
            }
        };
        if start.len() != self.delegate.columns.len() {
            return;
        }

        let anchor: Vec<Resolved<S::Field>> = self
            .delegate
            .columns
            .iter()
            .zip(&start)
            .map(|(column, width)| Resolved {
                spec: column.spec,
                width: *width,
                floor: column.floor,
            })
            .collect();
        let Some(widths) = stretch(&anchor, resize.column, to - resize.origin.get()) else {
            return;
        };

        let mut layout = self.delegate.layout.clone();
        layout.widths.extend(widths);
        self.delegate.set_layout(layout, cx);
        cx.emit(TableEvent::LayoutChanged);
        cx.notify();
    }

    fn move_column(&mut self, moved: &ColumnMove, to: Pixels, cx: &mut Context<Self>) {
        let target = shifted(
            &self.delegate.columns,
            moved.column,
            to - moved.origin.get(),
        );
        if self.moving == Some((moved.column, target)) {
            return;
        }
        self.moving = Some((moved.column, target));
        cx.notify();
    }

    fn finish_column_drag(&mut self, cx: &mut Context<Self>) {
        self.sizing = None;
        let Some((from, to)) = self.moving.take() else {
            return;
        };
        if from == to {
            cx.notify();
            return;
        }

        let mut layout = self.delegate.layout.clone();
        layout.order = reordered(&self.delegate.columns, from, to);
        self.delegate.set_layout(layout, cx);
        cx.emit(TableEvent::LayoutChanged);
        cx.notify();
    }

    fn header(&self, head: Pixels, top: Corners<Pixels>, cx: &mut Context<Self>) -> Div {
        let theme = *cx.theme();
        let last = self.delegate.columns.len().saturating_sub(1);
        let heads: Vec<_> = self
            .delegate
            .columns
            .iter()
            .enumerate()
            .map(|(ix, column)| {
                let sortable = column.spec.sortable;
                let room = match sortable {
                    true => SORT_ROOM,
                    false => Pixels::ZERO,
                };
                (
                    ix,
                    column.width,
                    self.delegate.inner_width(ix) - room,
                    column.spec.align,
                    column.spec.label(),
                    sortable,
                    self.delegate.direction(column.spec.field),
                    !column.spec.anchored,
                )
            })
            .collect();
        let marker = self.moving.and_then(|(from, to)| {
            let column = self.delegate.columns.get(to)?;
            (from != to).then(|| match to > from {
                true => self.delegate.offset(to) + column.width,
                false => self.delegate.offset(to),
            })
        });
        let handles: Vec<(usize, Pixels)> = self
            .delegate
            .columns
            .iter()
            .enumerate()
            .take(last)
            .filter(|(_, column)| !column.spec.anchored)
            .map(|(ix, _)| (ix, self.delegate.offset(ix + 1)))
            .collect();

        div()
            .relative()
            .flex()
            .flex_none()
            .h(head)
            .bg(theme.table_head)
            .rounded_tl(top.top_left)
            .rounded_tr(top.top_right)
            .border_b_1()
            .border_color(theme.table_row_border)
            .text_color(theme.table_head_foreground)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_column_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_column_drag(cx)),
            )
            .children(heads.into_iter().map(
                |(ix, width, inner, align, header, sortable, direction, movable)| {
                    let dragged = self.moving.is_some_and(|(from, _)| from == ix);
                    div()
                        .id(("head", ix))
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .w(width)
                        .h_full()
                        .px(PADDING)
                        .overflow_hidden()
                        .when(dragged, |this| this.bg(theme.table_active))
                        .when(sortable, |this| {
                            this.cursor_pointer()
                                .hover(move |style| style.text_color(theme.foreground))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                        if this.moving.is_some() || this.sizing.is_some() {
                                            return;
                                        }
                                        this.toggle_sort(ix, cx);
                                    }),
                                )
                        })
                        .when(movable, |this| {
                            this.on_drag_move(cx.listener(
                                move |this, event: &DragMoveEvent<ColumnMove>, _, cx| {
                                    let moved = event.drag(cx).clone();
                                    if moved.column != ix {
                                        return;
                                    }
                                    this.move_column(&moved, event.event.position.x, cx);
                                },
                            ))
                            .on_drag(
                                ColumnMove {
                                    column: ix,
                                    origin: StdCell::new(Pixels::ZERO),
                                },
                                |moved, _, window, cx| {
                                    moved.origin.set(window.mouse_position().x);
                                    cx.new(|_| Empty)
                                },
                            )
                        })
                        .child(frame(inner, align).child(header))
                        .when(sortable, |this| {
                            this.child(
                                svg()
                                    .path(icons::path(sort_icon(direction)))
                                    .size(px(12.))
                                    .flex_none()
                                    .text_color(match direction {
                                        Some(_) => theme.foreground,
                                        None => theme.table_head_foreground,
                                    }),
                            )
                        })
                },
            ))
            .children(
                handles
                    .into_iter()
                    .map(|(ix, edge)| resize_handle(ix, edge, cx)),
            )
            .when_some(marker, |this, marker| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(marker - px(1.))
                        .w(px(2.))
                        .bg(theme.foreground),
                )
            })
    }

    fn rows(&self, head: Pixels, row_height: Pixels, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = *cx.theme();
        let count = self.delegate.order.len();
        let first = self.viewport.first(head, row_height);
        let last = (first + self.viewport.rows(row_height)).min(count);
        let bottom = self.corners.bottom_left.max(self.corners.bottom_right);
        let marked = &self.delegate.marked;

        (first..last)
            .map(|display| {
                let row = self.delegate.row(display);
                let tail = display + 1 == count;
                let selected = marked.contains(&row);
                let playing = self.delegate.source.playing(row, cx);
                let cells: Vec<AnyElement> = (0..self.delegate.columns.len())
                    .map(|ix| {
                        let column = &self.delegate.columns[ix];
                        let cell = Cell {
                            field: column.spec.field,
                            width: self.delegate.inner_width(ix),
                            height: row_height,
                            align: column.spec.align,
                            display,
                            row,
                        };
                        self.delegate.source.cell(cell, cx)
                    })
                    .collect();

                div()
                    .id(("row", display))
                    .group(ROW_GROUP)
                    .absolute()
                    .top(head + row_height * display as f32)
                    .left_0()
                    .w_full()
                    .flex()
                    .items_center()
                    .h(row_height)
                    .when(tail, |this| {
                        this.rounded_bl(self.corners.bottom_left)
                            .rounded_br(self.corners.bottom_right)
                    })
                    .when(!tail || bottom == Pixels::ZERO, |this| {
                        this.border_b_1().border_color(theme.table_row_border)
                    })
                    .when(playing, |this| this.bg(theme.muted))
                    .when(selected, |this| this.bg(theme.table_active))
                    .when(!selected, |this| {
                        this.hover(move |style| style.bg(theme.table_hover))
                    })
                    .when_some(self.delegate.source.pin(row, cx), Pinnable::pin)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus.clone(), cx);
                            this.delegate.pick(
                                row,
                                event.modifiers.shift,
                                event.modifiers.secondary() && !event.modifiers.shift,
                            );
                            if event.click_count >= 2 {
                                cx.emit(TableEvent::DoubleClicked(display));
                            }
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            if !this.delegate.holds(row) {
                                this.delegate.pick(row, false, false);
                            }
                            let rows = this.delegate.picked();
                            let visible = this.delegate.visible();
                            if this
                                .delegate
                                .source
                                .context_menu(&rows, &visible, cx)
                                .is_some()
                            {
                                this.delegate.source.context_menu_will_open(&rows, cx);
                                window.focus(&this.focus.clone(), cx);
                                window.prevent_default();
                                cx.stop_propagation();
                                this.context_menu = Some((rows, event.position));
                                cx.notify();
                            }
                        }),
                    )
                    .children(cells)
                    .into_any_element()
            })
            .collect()
    }
}

fn resize_handle<S: TableSource>(
    ix: usize,
    edge: Pixels,
    cx: &mut Context<TableState<S>>,
) -> Stateful<Div> {
    div()
        .id(("grip", ix))
        .block_mouse_except_scroll()
        .absolute()
        .top_0()
        .bottom_0()
        .left(edge - GRIP / 2.)
        .w(GRIP)
        .cursor_col_resize()
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<ColumnResize>, _, cx: &mut Context<TableState<S>>| {
                let resize = event.drag(cx).clone();
                if resize.column != ix {
                    return;
                }
                this.resize(&resize, event.event.position.x, cx);
            },
        ))
        .on_drag(
            ColumnResize {
                column: ix,
                origin: StdCell::new(Pixels::ZERO),
            },
            |resize, _, window, cx| {
                resize.origin.set(window.mouse_position().x);
                cx.new(|_| Empty)
            },
        )
}

fn unpinned(corners: Corners<Pixels>, pinned: Pixels) -> Corners<Pixels> {
    Corners {
        top_left: (corners.top_left - pinned).max(Pixels::ZERO),
        top_right: (corners.top_right - pinned).max(Pixels::ZERO),
        ..Corners::default()
    }
}

fn radii(style: &StyleRefinement, rem: Pixels) -> Corners<Pixels> {
    let resolve = |length: Option<AbsoluteLength>| {
        length
            .map(|length| length.to_pixels(rem))
            .unwrap_or_default()
            .max(Pixels::ZERO)
    };

    Corners {
        top_left: resolve(style.corner_radii.top_left),
        top_right: resolve(style.corner_radii.top_right),
        bottom_right: resolve(style.corner_radii.bottom_right),
        bottom_left: resolve(style.corner_radii.bottom_left),
    }
}

fn sort_icon(direction: Option<Sort>) -> &'static str {
    match direction {
        Some(Sort::Ascending) => "icons/chevron-up.svg",
        Some(Sort::Descending) => "icons/chevron-down.svg",
        None => "icons/chevrons-up-down.svg",
    }
}

impl<S: TableSource> Render for TableState<S> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.delegate.measure(window, cx);

        let metrics = cx.theme().metrics;
        let backdrop = cx.theme().background;
        let row = snapped(metrics.row, window);
        let head = snapped(metrics.header, window);
        let height = self.height(head, row);
        let pinned = snapped(self.viewport.top.clamp(Pixels::ZERO, height - head), window);
        let top = unpinned(self.corners, pinned);
        let context_menu = self.context_menu.clone().and_then(|(rows, position)| {
            let visible = self.delegate.visible();
            self.delegate
                .source
                .context_menu(&rows, &visible, cx)
                .map(|menu| {
                    Popup::new(position, menu).on_close(cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        cx.notify();
                    }))
                })
        });

        div()
            .key_context(TABLE_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::deselect))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::remove))
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                if this.context_menu.is_some() {
                    return;
                }
                if this.delegate.selected.is_none() && this.delegate.marked.is_empty() {
                    return;
                }
                this.delegate.clear_selection();
                cx.notify();
            }))
            .relative()
            .w_full()
            .h(height)
            .children(self.rows(head, row, cx))
            .child(
                div()
                    .block_mouse_except_scroll()
                    .absolute()
                    .top(pinned)
                    .left_0()
                    .w_full()
                    .bg(backdrop)
                    .rounded_tl(top.top_left)
                    .rounded_tr(top.top_right)
                    .child(self.header(head, top, cx)),
            )
            .when_some(context_menu, |this, menu| this.child(menu))
    }
}

#[derive(IntoElement)]
pub struct Table<S: TableSource> {
    base: Div,
    state: Entity<TableState<S>>,
}

impl<S: TableSource> Styled for Table<S> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl<S: TableSource> InteractiveElement for Table<S> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl<S: TableSource> RenderOnce for Table<S> {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let corners = radii(self.base.style(), window.rem_size());
        self.state.update(cx, |state, _| state.corners = corners);

        self.base.child(self.state)
    }
}

pub fn table<S: TableSource>(state: &Entity<TableState<S>>) -> Table<S> {
    Table {
        base: div(),
        state: state.clone(),
    }
}

#[derive(Default)]
struct Shown(Option<Box<dyn Listing>>);

impl Global for Shown {}

pub fn show_listing(table: Box<dyn Listing>, cx: &mut App) {
    cx.default_global::<Shown>().0 = Some(table);
}

pub fn clear_listing(cx: &mut App) {
    if cx.has_global::<Shown>() {
        cx.global_mut::<Shown>().0 = None;
    }
}

pub fn shown_listing(cx: &App) -> Option<Box<dyn Listing>> {
    Some(cx.try_global::<Shown>()?.0.as_ref()?.boxed())
}

pub trait Listing {
    fn layout(&self, cx: &App) -> Layout;
    fn set_layout(&self, layout: Layout, cx: &mut App);
    fn sorting(&self, cx: &App) -> Option<Sorting>;
    fn set_sorting(&self, sorting: Option<Sorting>, cx: &mut App);
    fn sortables(&self, cx: &App) -> Vec<SortAxis>;
    fn cycle_sort(&self, column: &str, cx: &mut App);
    fn row_count(&self, cx: &App) -> usize;
    fn filters(&self, cx: &App) -> Vec<Filter>;
    fn filter(&self, change: FilterChange, cx: &mut App);
    fn filtering(&self, cx: &App) -> bool;
    fn toggles(&self, cx: &App) -> Vec<Toggle>;
    fn set_width(&self, width: Pixels, cx: &mut App);
    fn set_query(&self, query: &str, cx: &mut App);
    fn set_viewport(&self, viewport: Viewport, cx: &mut App);
    fn rebuild(&self, cx: &mut App);
    fn refresh(&self, cx: &mut App);
    fn element(&self) -> AnyElement;
    fn boxed(&self) -> Box<dyn Listing>;
    fn claim(&self, cx: &mut App) {
        show_listing(self.boxed(), cx);
    }
    fn select_next(&self, window: &mut Window, cx: &mut App);
    fn select_previous(&self, window: &mut Window, cx: &mut App);
    fn deselect(&self, cx: &mut App);
    fn activate(&self, cx: &mut App);
    fn remove(&self, cx: &mut App);
}

impl<S: TableSource> Listing for Entity<TableState<S>> {
    fn layout(&self, cx: &App) -> Layout {
        self.read(cx).delegate().layout().clone()
    }

    fn set_layout(&self, layout: Layout, cx: &mut App) {
        self.update(cx, |table, cx| {
            table.delegate_mut().set_layout(layout, cx);
            table.refresh(cx);
        });
    }

    fn sorting(&self, cx: &App) -> Option<Sorting> {
        self.read(cx).delegate().sorting()
    }

    fn set_sorting(&self, sorting: Option<Sorting>, cx: &mut App) {
        self.update(cx, |table, cx| {
            table.delegate_mut().set_sorting(sorting, cx);
            table.refresh(cx);
        });
    }

    fn sortables(&self, cx: &App) -> Vec<SortAxis> {
        self.read(cx).delegate().sortables()
    }

    fn cycle_sort(&self, column: &str, cx: &mut App) {
        let column = column.to_owned();
        self.update(cx, |table, cx| {
            table.delegate_mut().cycle(&column, cx);
            table.refresh(cx);
            cx.emit(TableEvent::SortChanged);
        });
    }

    fn toggles(&self, cx: &App) -> Vec<Toggle> {
        self.read(cx).delegate().toggles()
    }

    fn row_count(&self, cx: &App) -> usize {
        self.read(cx).delegate().row_count()
    }

    fn filters(&self, cx: &App) -> Vec<Filter> {
        let delegate = self.read(cx).delegate();
        delegate.source().filter_axes(delegate.query(), cx)
    }

    fn filter(&self, change: FilterChange, cx: &mut App) {
        self.update(cx, |table, cx| {
            if table.delegate_mut().source_mut().filter(change, cx) {
                table.delegate_mut().reorder(cx);
                table.refresh(cx);
            }
        });
    }

    fn filtering(&self, cx: &App) -> bool {
        let delegate = self.read(cx).delegate();
        !delegate.query().is_empty() || delegate.source().filtered(cx)
    }

    fn set_width(&self, width: Pixels, cx: &mut App) {
        self.update(cx, |table, cx| {
            table.delegate_mut().set_width(width, cx);
            table.refresh(cx);
        });
    }

    fn set_query(&self, query: &str, cx: &mut App) {
        self.update(cx, |table, cx| {
            table.delegate_mut().set_query(query, cx);
            table.refresh(cx);
        });
    }

    fn set_viewport(&self, viewport: Viewport, cx: &mut App) {
        self.update(cx, |table, _| table.set_viewport(viewport));
    }

    fn rebuild(&self, cx: &mut App) {
        self.update(cx, |table, cx| table.rebuild(cx));
    }

    fn refresh(&self, cx: &mut App) {
        self.update(cx, |table, cx| table.refresh(cx));
    }

    fn element(&self) -> AnyElement {
        table(self).into_any_element()
    }

    fn boxed(&self) -> Box<dyn Listing> {
        Box::new(self.clone())
    }

    fn select_next(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |table, cx| {
            window.focus(&table.focus.clone(), cx);
            table.step(1, window, cx);
        });
    }

    fn select_previous(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |table, cx| {
            window.focus(&table.focus.clone(), cx);
            table.step(-1, window, cx);
        });
    }

    fn deselect(&self, cx: &mut App) {
        self.update(cx, |table, cx| {
            table.delegate.clear_selection();
            table.context_menu = None;
            cx.notify();
        });
    }

    fn activate(&self, cx: &mut App) {
        self.update(cx, |table, cx| table.fire_activate(cx));
    }

    fn remove(&self, cx: &mut App) {
        self.update(cx, |table, cx| table.fire_remove(cx));
    }
}
