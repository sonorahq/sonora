//! Menus drawn by the app itself, since Linux has no native pop-up menu: a frosted panel of
//! rows anchored to a trigger or opened at the pointer, walked with the arrow keys, with
//! check marks, submenus that open beside their row, and an optional search field over the rows.

use crate::smooth::Smooth as _;
use std::rc::Rc;

use rune::{
    Cursor, El, Key, KeyEvent, ScrollSpec, TextEdit, TextStyle, Tx, col, el, icon, row, text,
};

use crate::glass::{GLASS_BLUR, blurring, popover_fill};
use crate::input::Input;
use crate::label::Weight;
use crate::metrics::{Text, tucked};
use crate::motion::Springs;
use crate::overlay::{self, Placement};
use crate::paint::{rems, shadow_md};
use crate::separator::Separator;
use crate::theme::Theme;

/// The gap a menu leaves between itself and the trigger it drops from.
pub const TRIGGER_GAP: f32 = 4.;
const BORDER: f32 = 1.;
/// The space between two rows.
const ROW_GAP: f32 = 2.;
const ICON: f32 = 14.;
/// How far a scrolling menu's edges fade while rows pass under them.
const FADE: f32 = 16.;
/// The check mark and the arrow of a row that opens a submenu.
const CHECK: &str = "✓";
const MORE: &str = "›";

/// What a row does when it is picked, after the menu's own bookkeeping.
type Pick<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
/// Opens or closes a submenu.
type Open<S> = Rc<dyn Fn(&mut S, bool)>;
/// Finds a menu's search in the window's state.
type Search<S> = Rc<dyn for<'a> Fn(&'a mut S) -> &'a mut MenuSearch>;

/// A search field over a menu's rows, kept in the window's state. The menu shows it as its
/// header, takes focus for it as it opens, and lets the arrow keys walk and Return pick the rows
/// that can be picked, so the owner only filters its items by [`MenuSearch::query`].
#[derive(Clone, Debug, Default)]
pub struct MenuSearch {
    pub edit: TextEdit,
    /// The place among the pickable rows the arrow keys moved to.
    cursor: Option<usize>,
}

impl MenuSearch {
    pub fn new() -> Self {
        Self::default()
    }

    /// The field's text, trimmed and lowercased for matching.
    pub fn query(&self) -> String {
        self.edit.text.trim().to_lowercase()
    }

    /// Empties the field and forgets the row the keys reached, for a menu about to open afresh.
    pub fn reset(&mut self) {
        self.edit.clear();
        self.cursor = None;
    }

    /// The place among `count` pickable rows the cursor is on: the first match until the arrow
    /// keys move it, and nowhere while the field is empty.
    pub fn place(&self, count: usize) -> Option<usize> {
        let place = self
            .cursor
            .or_else(|| (!self.query().is_empty()).then_some(0))?;
        (count > 0).then(|| place.min(count - 1))
    }

    /// Moves the cursor `step` rows through `count`, wrapping at the ends.
    fn walk(&mut self, count: usize, step: isize) {
        if count == 0 {
            return;
        }
        self.cursor = Some(match self.place(count) {
            Some(place) => (place as isize + step).rem_euclid(count as isize) as usize,
            None if step > 0 => 0,
            None => count - 1,
        });
    }
}

/// A submenu and how its row opens it. Whether it shows is the owner's state.
struct Submenu<S> {
    menu: Box<Menu<S>>,
    open: bool,
    toggle: Open<S>,
}

/// One row of a [`Menu`], or a separator, or a block of custom content.
pub struct MenuItem<S> {
    key: String,
    label: String,
    detail: Option<El>,
    selected: bool,
    /// Whether a keyboard cursor the owner tracks rests on the row: painted like a selected one,
    /// without the check.
    highlighted: bool,
    /// Whether the row toggles something, and whether that is on.
    checked: Option<bool>,
    disabled: bool,
    separator: bool,
    content: Option<El>,
    icon: Option<String>,
    leading: Option<El>,
    trailing: Option<El>,
    tooltip: Option<String>,
    press: Option<Pick<S>>,
    submenu: Option<Submenu<S>>,
    /// Whether picking the row leaves the menu open, for a row that toggles something in place.
    stays: bool,
}

impl<S: 'static> MenuItem<S> {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            detail: None,
            selected: false,
            highlighted: false,
            checked: None,
            disabled: false,
            separator: false,
            content: None,
            icon: None,
            leading: None,
            trailing: None,
            tooltip: None,
            press: None,
            submenu: None,
            stays: false,
        }
    }

    pub fn separator(key: impl Into<String>) -> Self {
        Self {
            separator: true,
            disabled: true,
            ..Self::new(key, "")
        }
    }

    /// A row that holds `content` rather than a choice, such as a slider inside the menu.
    pub fn content(key: impl Into<String>, content: El) -> Self {
        Self {
            content: Some(content),
            disabled: true,
            ..Self::new(key, "")
        }
    }

    /// Makes the row a toggle, read as checked or not, with a check after the label while on.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// The current choice of a picker: painted, checked, and where keyboard focus starts.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Marks the row the owner's own keyboard cursor is on. Unlike `selected`, no check.
    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    /// A second element under the label, which wraps at the menu's width.
    pub fn detail(mut self, detail: El) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    /// Text read after the label and shown over the row while the pointer rests on it, already
    /// resolved.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    pub fn icon(mut self, path: impl Into<String>) -> Self {
        self.icon = Some(path.into());
        self
    }

    /// Draws `leading` before the label, such as a cover or a small picture of the choice.
    pub fn leading(mut self, leading: El) -> Self {
        self.leading = Some(leading);
        self
    }

    /// Draws `trailing` at the end of the row, such as badges or a small button.
    pub fn trailing(mut self, trailing: El) -> Self {
        self.trailing = Some(trailing);
        self
    }

    pub fn on_press(self, f: impl Fn(&mut S) + 'static) -> Self {
        self.on_press_tx(move |s: &mut S, _: &mut Tx| f(s))
    }

    pub fn on_press_tx(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.press = Some(Rc::new(f));
        self
    }

    /// Keeps the menu open after the row is picked, for a checked row that toggles in place.
    pub fn stays_open(mut self) -> Self {
        self.stays = true;
        self
    }

    /// Opens `menu` beside the row while `open`. The row calls `toggle` with true when the
    /// pointer comes to it or the Right arrow is pressed on it, and with false when the pointer
    /// moves to a sibling row or the Left arrow goes back out of the submenu.
    pub fn submenu(
        mut self,
        menu: Menu<S>,
        open: bool,
        toggle: impl Fn(&mut S, bool) + 'static,
    ) -> Self {
        self.submenu = Some(Submenu {
            menu: Box::new(menu),
            open,
            toggle: Rc::new(toggle),
        });
        self
    }

    fn pickable(&self) -> bool {
        !self.disabled && !self.separator && self.content.is_none() && self.press.is_some()
    }
}

/// A menu's panel. Mount it with [`Menu::build`], which anchors it to a trigger keyed elsewhere
/// in the view or opens it at a point, and pushes it and any open submenus into the layer.
pub struct Menu<S> {
    key: String,
    items: Vec<MenuItem<S>>,
    /// What a screen reader calls the menu, read as focus enters it.
    name: Option<String>,
    header: Option<El>,
    search: Option<(MenuSearch, Search<S>, String)>,
    dismiss: Option<Pick<S>>,
    picked: Option<Pick<S>>,
    width: Option<f32>,
    min_width: Option<f32>,
    max_height: Option<f32>,
    anchor: Option<(String, Placement, f32)>,
    at: Option<(f32, f32)>,
    /// Back out of a submenu with the Left arrow: the parent row's key and its toggle.
    parent: Option<(String, Open<S>)>,
}

impl<S: 'static> Menu<S> {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            items: Vec::new(),
            name: None,
            header: None,
            search: None,
            dismiss: None,
            picked: None,
            width: None,
            min_width: None,
            max_height: None,
            anchor: None,
            at: None,
            parent: None,
        }
    }

    pub fn item(mut self, item: MenuItem<S>) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = MenuItem<S>>) -> Self {
        self.items.extend(items);
        self
    }

    /// Puts `items` before the ones already added, with a separator after them when the menu
    /// held anything.
    pub fn lead(mut self, items: impl IntoIterator<Item = MenuItem<S>>) -> Self {
        let mut led: Vec<MenuItem<S>> = items.into_iter().collect();
        if !led.is_empty() && !self.items.is_empty() {
            led.push(MenuItem::separator("lead-separator"));
        }
        self.items.splice(0..0, led);
        self
    }

    /// Names the menu for screen readers, usually after the button or setting that opens it.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// An element over the rows, such as a title.
    pub fn header(mut self, header: El) -> Self {
        self.header = Some(header);
        self
    }

    /// A search field over the rows, showing `search` from the window's state with the
    /// placeholder `hint`. The field takes focus as the menu opens; Up and Down walk the rows
    /// that can be picked and Return picks the one reached.
    pub fn search(
        mut self,
        search: &MenuSearch,
        hint: impl Into<String>,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut MenuSearch + 'static,
    ) -> Self {
        self.search = Some((search.clone(), Rc::new(access), hint.into()));
        self
    }

    /// Runs when the menu is dismissed without a pick: Escape or a press outside it.
    pub fn on_dismiss(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.dismiss = Some(Rc::new(move |s: &mut S, _: &mut Tx| f(s)));
        self
    }

    /// Runs after any row is picked that does not stay open, after the row's own handler. A
    /// popover passes its close here.
    pub fn on_pick(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.picked = Some(Rc::new(f));
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Caps the panel at `height` and scrolls what does not fit. A menu is always at most as
    /// tall as the window leaves it beside its trigger.
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height);
        self
    }

    /// Anchors the panel to the element keyed `key`, on its `placement` side, `gap` away.
    pub fn anchor(mut self, key: impl Into<String>, placement: Placement, gap: f32) -> Self {
        self.anchor = Some((key.into(), placement, gap));
        self
    }

    /// Opens the panel with its top left corner at a window point, as a context menu does.
    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.at = Some((x, y));
        self
    }

    /// The panel and its open submenus, pushed into `layer`. A menu with a dismiss handler
    /// holds the keyboard while open: Escape and a press outside close it, and focus goes back
    /// to its trigger when it does.
    pub fn build(mut self, theme: &Theme, layer: &mut Vec<El>) {
        let key = self.key.clone();
        let mut placed = None;
        let (anchor, placement, gap) = match (self.at.take(), self.anchor.take()) {
            (Some((x, y)), _) => {
                let point = format!("{key}-point");
                layer.push(el().key(point.clone()).absolute(x, y).size(0., 0.));
                (point, Placement::BottomStart, 0.)
            }
            (None, Some((anchor, placement, gap))) => {
                placed = Some(anchor.clone());
                (anchor, placement, gap)
            }
            (None, None) => (
                format!("{key}-trigger"),
                Placement::BottomStart,
                TRIGGER_GAP,
            ),
        };
        let dismiss = self.dismiss.clone();
        let mut subs = Vec::new();
        let mut surface = self.surface(theme, &mut subs);
        if let Some(trigger) = placed {
            surface = surface.focus_return(trigger);
        }
        // Submenus ride inside the panel, so the modal that holds the keyboard holds theirs too.
        surface = surface.children(subs);
        if let Some(dismiss) = dismiss {
            surface = surface.modal_tx(true, move |s: &mut S, tx: &mut Tx| dismiss(s, tx));
        }
        layer.push(overlay::anchor(surface, &anchor, placement, gap));
    }

    /// The panel itself, with its open submenus pushed into `layer`, since they anchor to rows
    /// inside it.
    fn surface(self, theme: &Theme, layer: &mut Vec<El>) -> El {
        let Menu {
            key,
            items,
            name,
            header,
            search,
            picked,
            width,
            min_width,
            max_height,
            parent,
            ..
        } = self;
        let pad = rems(theme, 0.25);
        let radius = tucked(theme.radius, theme.font_size);
        let pickable: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.pickable())
            .map(|(place, _)| place)
            .collect();
        let cursor = search
            .as_ref()
            .and_then(|(search, _, _)| search.place(pickable.len()))
            .map(|place| pickable[place]);
        let choices = items
            .iter()
            .filter(|item| !item.separator && item.content.is_none())
            .count();
        let toggles: Vec<(String, Open<S>)> = items
            .iter()
            .filter_map(|item| {
                let sub = item.submenu.as_ref()?;
                Some((item.key.clone(), sub.toggle.clone()))
            })
            .collect();
        let toggles = Rc::new(toggles);

        let cursor_press = cursor.and_then(|place| {
            let item = &items[place];
            let press = item.press.clone()?;
            let after = picked.clone().filter(|_| !item.stays);
            Some((press, after))
        });
        let mut position = 0;
        let mut rows = Vec::with_capacity(items.len());
        for (place, item) in items.into_iter().enumerate() {
            if item.separator {
                rows.push(
                    row()
                        .key(format!("{key}-{}", item.key))
                        .w_full()
                        .px(rems(theme, 0.5))
                        .py(rems(theme, 0.25))
                        .child(Separator::horizontal().build(theme))
                        .a11y_hidden(),
                );
                continue;
            }
            if let Some(content) = item.content {
                rows.push(
                    col()
                        .key(format!("{key}-{}", item.key))
                        .w_full()
                        .min_w(0.)
                        .px(rems(theme, 0.75))
                        .py(rems(theme, 0.25))
                        .child(content),
                );
                continue;
            }
            position += 1;
            let row_key = format!("{key}-{}", item.key);
            let lit = item.selected || item.highlighted || cursor == Some(place);
            rows.push(menu_row(
                theme,
                item,
                RowPlace {
                    key: row_key,
                    position,
                    count: choices,
                    lit,
                    radius,
                },
                picked.clone(),
                toggles.clone(),
                parent.clone(),
                layer,
            ));
        }

        let body = col()
            .key(format!("{key}-rows"))
            .w_full()
            .gap(ROW_GAP)
            .children(rows);
        let scroll = col()
            .key(format!("{key}-scroll"))
            .w_full()
            .shrink()
            .min_h(0.)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .smooth::<S>(&format!("{key}-scroll"))
            .scroll_edge_fade(FADE)
            .scroll_margin(FADE)
            .child(body);

        let mut panel = col()
            .key(format!("{key}-menu"))
            .role("menu")
            .focus_group()
            .min_h(0.)
            .padding(pad)
            .gap(pad)
            .radius(theme.radius)
            .border(BORDER, theme.border.color())
            .fill(popover_fill(theme).color())
            .motion(Springs::CONTROL);
        panel = shadow_md(panel);
        if blurring(theme) {
            panel = panel.backdrop_blur(GLASS_BLUR);
        }
        if let Some(name) = name {
            panel = panel.label(name);
        }
        if let Some(width) = width {
            panel = panel.w(width);
        }
        if let Some(width) = min_width {
            panel = panel.min_w(width);
        }
        if let Some(height) = max_height {
            panel = panel.max_h(height);
        }
        if let Some(header) = header {
            panel = panel.child(row().w_full().child(header));
        }
        if let Some((search, access, hint)) = search {
            let count = pickable.len();
            let walk = access.clone();
            let edits = access.clone();
            let changed = access.clone();
            let mut field = Input::new("search", &search.edit, hint, move |s: &mut S| {
                &mut edits(s).edit
            })
            .compact()
            .tucked()
            .autofocus()
            .on_change(move |s: &mut S, _: &mut Tx| changed(s).cursor = None)
            .on_key(
                move |s: &mut S, event: &KeyEvent, _: &mut Tx| match event.key {
                    Key::Down => {
                        walk(s).walk(count, 1);
                        true
                    }
                    Key::Up => {
                        walk(s).walk(count, -1);
                        true
                    }
                    _ => false,
                },
            );
            if let Some((press, after)) = cursor_press {
                field = field.on_submit(move |s: &mut S, tx: &mut Tx| {
                    press(s, tx);
                    if let Some(after) = &after {
                        after(s, tx);
                    }
                });
            }
            panel = panel.child(field.build(theme));
        }
        panel.child(scroll)
    }
}

/// Where a row sits and how it is painted.
struct RowPlace {
    key: String,
    position: usize,
    count: usize,
    lit: bool,
    radius: f32,
}

/// One pickable row of a menu, with its submenu pushed into `layer` when open.
#[allow(clippy::too_many_arguments)]
fn menu_row<S: 'static>(
    theme: &Theme,
    item: MenuItem<S>,
    place: RowPlace,
    picked: Option<Pick<S>>,
    toggles: Rc<Vec<(String, Open<S>)>>,
    parent: Option<(String, Open<S>)>,
    layer: &mut Vec<El>,
) -> El {
    let MenuItem {
        key: item_key,
        label,
        detail,
        selected,
        checked,
        disabled,
        icon: glyph,
        leading,
        trailing,
        tooltip,
        press,
        submenu,
        stays,
        ..
    } = item;
    let RowPlace {
        key,
        position,
        count,
        lit,
        radius,
    } = place;
    let ink = match disabled {
        true => theme.muted_foreground,
        false => theme.popover_foreground,
    };
    let style = TextStyle::new(theme.text(Text::Body), Weight::NORMAL);
    let detailed = detail.is_some();
    // No grow: a grown row measures nothing, and the panel sizes itself to its widest row.
    let mut words = col().shrink().min_w(0.).gap(rems(theme, 0.25)).child(
        text(label.clone())
            .font(style)
            .color(ink.color())
            .truncate()
            .a11y_hidden(),
    );
    if let Some(detail) = detail {
        words = words.child(detail);
    }
    let mut lead = row().shrink().min_w(0.).center_y().gap(rems(theme, 0.5));
    if let Some(leading) = leading {
        lead = lead.child(leading);
    } else if let Some(path) = glyph {
        lead = lead.child(
            icon(icons::get(&path), ICON)
                .color(ink.color())
                .a11y_hidden(),
        );
    }
    lead = lead.child(words);

    let mark = |glyph: &str| text(glyph).font(style).color(ink.color()).a11y_hidden();
    let mut row = row()
        .key(key.clone())
        .role(match checked {
            Some(_) => "menuitemcheckbox",
            None => "menuitem",
        })
        .label(label)
        .position_in_set(position, count)
        .w_full()
        .min_w(0.)
        .center_y()
        .justify_between()
        .gap(rems(theme, 0.75))
        .px(rems(theme, 0.75))
        .py(match detailed {
            true => rems(theme, 0.5),
            false => rems(theme, 0.25),
        })
        .radius(radius)
        .child(lead);
    if let Some(on) = checked {
        row = row.checked(on);
    }
    if let Some(tip) = tooltip {
        row = row.description(tip.clone()).help(tip);
    }
    if let Some(trailing) = trailing {
        row = row.child(trailing);
    }
    if selected || checked == Some(true) {
        row = row.child(mark(CHECK));
    }
    if submenu.is_some() {
        row = row.child(mark(MORE));
    }
    row = match lit {
        true => row.fill(theme.secondary_active.color()),
        false => row,
    };
    if !disabled {
        row = row
            .focusable(true)
            .focus_selected(selected)
            .focus_highlight(theme.secondary_hover.color())
            .cursor(Cursor::Pointer);
    } else {
        row = row.disabled(true);
    }

    let opens = submenu.as_ref().map(|sub| sub.toggle.clone());
    let first = submenu.as_ref().and_then(|sub| {
        sub.menu
            .items
            .iter()
            .find(|item| item.pickable() || item.submenu.is_some())
            .map(|item| format!("{}-{}", sub.menu.key, item.key))
    });
    // Moving onto a row opens its own submenu and closes its siblings', as a menu does.
    if !toggles.is_empty() && !disabled {
        let mine = item_key.clone();
        let siblings = toggles.clone();
        row = row.on_hover_change(move |s: &mut S, hovered: bool, _: &mut Tx| {
            if !hovered {
                return;
            }
            for (sibling, toggle) in siblings.iter() {
                toggle(s, *sibling == mine);
            }
        });
    }
    // Right goes into a submenu and Left back out of one, as the arrows walk a menu bar.
    if opens.is_some() || parent.is_some() {
        let opens = opens.clone();
        row = row.on_key(
            move |s: &mut S, event: &KeyEvent, tx: &mut Tx| match event.key {
                Key::Right => match (&opens, &first) {
                    (Some(open), Some(first)) => {
                        open(s, true);
                        tx.focus(first.clone(), true);
                        true
                    }
                    _ => false,
                },
                Key::Left => match &parent {
                    Some((parent_key, back)) => {
                        back(s, false);
                        tx.focus(parent_key.clone(), true);
                        true
                    }
                    None => false,
                },
                _ => false,
            },
        );
    }
    if let Some(Submenu { menu, open, toggle }) = submenu {
        let opens = toggle.clone();
        row = row.expanded(open).on_press(move |s: &mut S| opens(s, true));
        if open {
            let mut menu = *menu;
            if menu.picked.is_none() {
                menu.picked = picked.clone();
            }
            menu.parent = Some((key.clone(), toggle));
            // a deeper submenu anchors to a row of this one, so it mounts after it
            let mut deeper = Vec::new();
            let surface = menu.surface(theme, &mut deeper);
            let pad = rems(theme, 0.25) + BORDER;
            layer.push(overlay::anchor(
                surface.offset(0., -pad),
                &key,
                Placement::RightStart,
                0.,
            ));
            layer.extend(deeper);
        }
        return row;
    }
    match (press, disabled) {
        (Some(press), false) => {
            let after = picked.filter(|_| !stays);
            row.on_press_tx(move |s: &mut S, tx: &mut Tx| {
                press(s, tx);
                if let Some(after) = &after {
                    after(s, tx);
                }
            })
        }
        _ => row,
    }
}
