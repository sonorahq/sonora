use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    Anchor, AnyElement, AnyWindowHandle, App, Bounds, ClickEvent, Div, ElementId, Entity, Global,
    Interactivity, Pixels, Point, ScrollWheelEvent, SharedString, Size, Stateful, StyleRefinement,
    Window, anchored, deferred, div, point, px, svg,
};

use crate::Artwork;
use crate::metrics::snapped;
use crate::motion::Rising as _;
use crate::scrollbar::Scrollbar;
use crate::separator::Separator;
use crate::shield::Shield;
use crate::theme::ActiveTheme as _;

pub const MENU_CONTEXT: &str = "Menu";

const ESCAPE_KEY: &str = "escape";

const SUBMENU_CLOSE_DELAY: Duration = Duration::from_millis(160);
const SUBMENU_FALLBACK_WIDTH: Pixels = px(236.);
const SUBMENU_TOP: Pixels = px(-14.);
const WINDOW_MARGIN: Pixels = px(8.);
pub(crate) const TRIGGER_GAP: Pixels = px(4.);
const PANEL_SLACK: Pixels = px(6.);
const SAFE_X: Pixels = px(6.);
const SAFE_Y: Pixels = px(12.);
const NEAR: usize = Near::Bar as usize + 1;

type Press = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type Close = Rc<dyn Fn(&(), &mut Window, &mut App) + 'static>;
type Action = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(Default)]
struct Escape(Rc<RefCell<Option<Close>>>);

impl Global for Escape {}

#[derive(Clone, Default)]
pub(crate) struct Trigger(Rc<Cell<Option<Bounds<Pixels>>>>);

impl Trigger {
    pub(crate) fn observe(&self, bounds: Vec<Bounds<Pixels>>) {
        self.0
            .set(bounds.into_iter().reduce(|one, other| one.union(&other)));
    }

    fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.0.get()
    }

    fn contains(&self, position: Point<Pixels>, slack: Pixels) -> bool {
        self.0
            .get()
            .is_some_and(|bounds| grown(bounds, slack, slack).contains(&position))
    }
}

#[derive(Clone, Copy)]
enum Near {
    Item,
    Gap,
    Panel,
    Bar,
}

#[derive(Clone, Default)]
pub struct SubmenuState {
    open: Rc<Cell<bool>>,
    generation: Rc<Cell<u64>>,
    near: Rc<Cell<[bool; NEAR]>>,
    flip: Rc<Cell<Option<bool>>>,
    menu_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    panel_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl SubmenuState {
    fn is_open(&self) -> bool {
        self.open.get()
    }

    fn touched(&self) -> bool {
        self.near.get().iter().any(|there| *there)
    }

    fn near(&self, place: Near, hovered: bool, window: AnyWindowHandle, cx: &mut App) {
        let mut near = self.near.get();
        if near[place as usize] == hovered {
            return;
        }
        near[place as usize] = hovered;
        self.near.set(near);

        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);

        if self.touched() {
            if !self.open.replace(true) {
                cx.refresh_windows();
            }
            return;
        }

        let state = self.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(SUBMENU_CLOSE_DELAY).await;
            let inside = cx.update(|cx| {
                cx.update_window(window, |_, window, _| state.covers(window.mouse_position()))
                    .unwrap_or(false)
            });
            cx.update(|cx| {
                if state.generation.get() == generation
                    && !state.touched()
                    && !inside
                    && state.open.replace(false)
                {
                    state.flip.set(None);
                    cx.refresh_windows();
                }
            });
        })
        .detach();
    }

    fn measure_panel(&self, bounds: Bounds<Pixels>) {
        self.panel_bounds.set(Some(grown(bounds, SAFE_X, SAFE_Y)));
    }

    fn measure_menu(&self, bounds: Bounds<Pixels>) {
        self.menu_bounds.set(Some(bounds));
    }

    /// Whether the submenu opens to the left of its menu. Decided once from the panel width
    /// measured last time it was open, or a guess before that; `measure_reach` corrects it.
    fn flipped(&self, viewport_width: Pixels) -> bool {
        if let Some(flip) = self.flip.get() {
            return flip;
        }
        let Some(menu) = self.menu_bounds.get() else {
            return false;
        };
        let width = self
            .panel_bounds
            .get()
            .map(|bounds| bounds.size.width)
            .unwrap_or(SUBMENU_FALLBACK_WIDTH);
        let flip = flips(menu, width, viewport_width);
        self.flip.set(Some(flip));
        flip
    }

    /// Re-decides the side once the submenu's real width is known, so a wrong guess flips it
    /// and a submenu too wide for either side lands on the roomier one.
    fn measure_reach(&self, bounds: Bounds<Pixels>, window: &Window, cx: &mut App) {
        let Some(menu) = self.menu_bounds.get() else {
            return;
        };
        let flip = flips(menu, bounds.size.width, window.viewport_size().width);
        if self.flip.replace(Some(flip)) == Some(flip) {
            return;
        }
        cx.refresh_windows();
    }

    fn covers(&self, position: Point<Pixels>) -> bool {
        self.panel_bounds
            .get()
            .is_some_and(|bounds| bounds.contains(&position))
    }

    fn contains(&self, position: Point<Pixels>) -> bool {
        self.is_open() && self.covers(position)
    }

    pub fn reset(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.open.set(false);
        self.near.set([false; NEAR]);
        self.flip.set(None);
        self.menu_bounds.set(None);
        self.panel_bounds.set(None);
    }
}

struct Submenu {
    menu: Box<Menu>,
    state: SubmenuState,
}

pub struct MenuItem {
    id: ElementId,
    label: SharedString,
    detail: Option<AnyElement>,
    selected: bool,
    checked: bool,
    disabled: bool,
    separator: bool,
    content: Option<AnyElement>,
    face: Option<SharedString>,
    icon: Option<&'static str>,
    artwork: Option<Option<SharedString>>,
    press: Option<Press>,
    submenu: Option<Submenu>,
}

impl FluentBuilder for MenuItem {}

impl MenuItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: None,
            selected: false,
            checked: false,
            face: None,
            disabled: false,
            separator: false,
            content: None,
            icon: None,
            artwork: None,
            press: None,
            submenu: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn separator(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: SharedString::default(),
            detail: None,
            selected: false,
            checked: false,
            disabled: true,
            separator: true,
            content: None,
            face: None,
            icon: None,
            artwork: None,
            press: None,
            submenu: None,
        }
    }

    pub fn detail(mut self, detail: impl IntoElement) -> Self {
        self.detail = Some(detail.into_any_element());
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self.disabled = true;
        self
    }

    pub fn face(mut self, family: impl Into<SharedString>) -> Self {
        self.face = Some(family.into());
        self
    }

    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub fn icon(mut self, path: &'static str) -> Self {
        self.icon = Some(path);
        self
    }

    pub fn artwork(mut self, url: Option<impl Into<SharedString>>) -> Self {
        self.artwork = Some(url.map(Into::into));
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.press = Some(Box::new(handler));
        self
    }

    pub fn submenu(mut self, menu: Menu, state: SubmenuState) -> Self {
        let mut menu = menu;
        menu.hover_guard = Some(state.clone());
        self.submenu = Some(Submenu {
            menu: Box::new(menu),
            state,
        });
        self
    }
}

#[derive(IntoElement)]
pub struct Menu {
    base: Stateful<Div>,
    items: Vec<MenuItem>,
    dismiss: Option<Close>,
    action: Option<Action>,
    priority: usize,
    deferred: bool,
    scrollbar: Option<Entity<Scrollbar>>,
    header: Option<AnyElement>,
    hover_guard: Option<SubmenuState>,
    trigger: Option<Trigger>,
}

impl Menu {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            items: Vec::new(),
            dismiss: None,
            action: None,
            priority: 1,
            deferred: true,
            scrollbar: None,
            header: None,
            hover_guard: None,
            trigger: None,
        }
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = MenuItem>) -> Self {
        self.items.extend(items);
        self
    }

    pub(crate) fn trigger(mut self, trigger: Trigger) -> Self {
        self.trigger = Some(trigger);
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.dismiss = Some(Rc::new(handler));
        self
    }

    pub(crate) fn on_action(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some(Rc::new(handler));
        self
    }

    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }

    pub fn scrollbar(mut self, scrollbar: Entity<Scrollbar>) -> Self {
        self.scrollbar = Some(scrollbar);
        self
    }

    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }

    pub(crate) fn inline(mut self) -> Self {
        self.deferred = false;
        self
    }
}

impl Styled for Menu {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Menu {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Menu {}

impl RenderOnce for Menu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            items,
            dismiss,
            action,
            priority,
            deferred: should_defer,
            scrollbar,
            header,
            hover_guard,
            trigger,
        } = self;

        if let (Some(scrollbar), Some(guard)) = (scrollbar.as_ref(), hover_guard.clone()) {
            scrollbar.update(cx, |scrollbar, _| {
                scrollbar.set_hover_guard(move |hovered, window, cx| {
                    guard.near(Near::Bar, hovered, window, cx)
                });
            });
        }

        let theme = *cx.theme();
        let overrides = std::mem::take(base.style());
        let panel = Trigger::default();
        let nested = hover_guard.is_some();
        let dismiss_guards: Vec<_> = items
            .iter()
            .filter_map(|item| item.submenu.as_ref().map(|submenu| submenu.state.clone()))
            .collect();
        let bounds_guards = dismiss_guards.clone();
        let viewport_width = window.viewport_size().width;
        let tucked = crate::metrics::tucked(theme.radius, window);
        let dismiss_for_items = dismiss.clone();

        let rows = items.into_iter().map(move |item| {
            let MenuItem {
                id,
                label,
                detail,
                selected,
                checked,
                disabled,
                separator,
                content,
                face,
                icon,
                artwork,
                press,
                submenu,
            } = item;

            if let Some(content) = content {
                return div()
                    .id(id)
                    .flex()
                    .w_full()
                    .min_w_0()
                    .flex_col()
                    .px_3()
                    .py_1()
                    .child(content)
                    .into_any_element();
            }

            if separator {
                return Separator::horizontal().mx_2().my_1().into_any_element();
            }
            let action = action.clone();
            let press_action = action.clone();
            let press_dismiss = dismiss_for_items.clone();
            let submenu_state = submenu.as_ref().map(|submenu| submenu.state.clone());
            let has_artwork = artwork.is_some();
            let detailed = detail.is_some();

            div()
                .id(id)
                .relative()
                .flex()
                .w_full()
                .min_w_0()
                .items_center()
                .justify_between()
                // The gap is part of the row's own width, so a menu widens rather than letting
                // a trailing mark crowd the label.
                .gap_3()
                .px_3()
                .when_else(detailed, |this| this.py_2(), |this| this.py_1())
                .rounded(tucked)
                .when_else(
                    disabled,
                    |this| this.text_color(theme.muted_foreground).cursor_default(),
                    |this| this.cursor_pointer(),
                )
                .when(selected, |this| this.bg(theme.secondary_active))
                .when(!disabled, |this| {
                    this.hover(move |this| this.bg(theme.secondary_hover))
                })
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .when_some(artwork, |this, artwork| {
                            this.child(Artwork::new(artwork).size(px(20.)).flex_none())
                        })
                        .when_some(icon.filter(|_| !has_artwork), |this, icon| {
                            this.child(
                                svg()
                                    .path(icons::path(icon))
                                    .size(px(14.))
                                    .flex_none()
                                    .text_color(if disabled {
                                        theme.muted_foreground
                                    } else {
                                        theme.popover_foreground
                                    }),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .truncate()
                                        .when_some(face, |this, family| {
                                            this.font(gpui::font(family))
                                        })
                                        .child(label),
                                )
                                .when_some(detail, |this, detail| this.child(detail)),
                        ),
                )
                .when(selected || checked, |this| {
                    this.child(div().flex_none().child("✓"))
                })
                .when(submenu.is_some(), |this| {
                    this.child(div().flex_none().child("›"))
                })
                .when_some(submenu_state, |this, state| {
                    this.on_hover(move |hovered, window, cx| {
                        state.near(Near::Item, *hovered, window.window_handle(), cx)
                    })
                })
                .when_some(press, |this, press| {
                    let dismiss = press_dismiss.clone();
                    this.on_click(move |event, window, cx| {
                        if let Some(dismiss) = dismiss.as_ref() {
                            dismiss(&(), window, cx);
                        }
                        if let Some(action) = press_action.as_ref() {
                            action(event, window, cx);
                        }
                        press(event, window, cx);
                    })
                })
                .when_some(submenu, |this, mut submenu| {
                    if submenu.menu.action.is_none() {
                        submenu.menu.action = action.clone();
                    }
                    let gap_state = submenu.state.clone();
                    let reach_state = submenu.state.clone();
                    match submenu.state.is_open() {
                        false => this,
                        true => this.child({
                            let flip_left = submenu.state.flipped(viewport_width);
                            let panel = div()
                                .absolute()
                                .top(SUBMENU_TOP)
                                .w(px(0.))
                                .when(flip_left, |this| this.right_full())
                                .when(!flip_left, |this| this.left_full())
                                .on_children_prepainted(move |bounds, window, cx| {
                                    if let Some(bounds) =
                                        bounds.into_iter().reduce(|one, other| one.union(&other))
                                    {
                                        reach_state.measure_reach(bounds, window, cx);
                                    }
                                })
                                .child(
                                    anchored()
                                        .anchor(match flip_left {
                                            true => Anchor::TopRight,
                                            false => Anchor::TopLeft,
                                        })
                                        .snap_to_window_with_margin(WINDOW_MARGIN)
                                        .child(
                                            div()
                                                .id("submenu-safe-area")
                                                .occlude()
                                                .pt_3()
                                                .pb_3()
                                                .when(flip_left, |this| this.pl_3().pr_1())
                                                .when(!flip_left, |this| this.pl_1().pr_3())
                                                .on_hover(move |hovered, window, cx| {
                                                    gap_state.near(
                                                        Near::Gap,
                                                        *hovered,
                                                        window.window_handle(),
                                                        cx,
                                                    )
                                                })
                                                .child(submenu.menu.inline().relative()),
                                        ),
                                );
                            deferred(panel).with_priority(priority + 1)
                        }),
                    }
                })
                .into_any_element()
        });

        let content = match scrollbar.as_ref() {
            Some(scrollbar) => {
                scrollbar.read(cx).sync();
                let gliding = scrollbar.clone();

                div()
                    .id("menu-scroll-content")
                    .flex()
                    .flex_1()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(scrollbar.read(cx).scroll())
                    .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                        if event.delta.precise() {
                            return;
                        }
                        gliding.update(cx, |bar, _| bar.nudge(window));
                    })
                    .children(rows)
                    .into_any_element()
            }
            None => div()
                .flex()
                .flex_col()
                .on_children_prepainted({
                    let panel = panel.clone();
                    move |bounds, _, _| panel.observe(bounds)
                })
                .gap(px(2.))
                .children(rows)
                .into_any_element(),
        };
        let body = match scrollbar {
            Some(scrollbar) => div()
                .relative()
                .flex()
                .flex_1()
                .w_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .on_children_prepainted({
                    let panel = panel.clone();
                    move |bounds, _, _| panel.observe(bounds)
                })
                .child(content)
                .child(scrollbar)
                .into_any_element(),
            None => content,
        };
        let body = match header {
            Some(header) => div()
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .min_w_0()
                .min_h_0()
                .gap_1()
                .on_children_prepainted({
                    let panel = panel.clone();
                    move |bounds, _, _| panel.observe(bounds)
                })
                .child(div().w_full().py_1().child(header))
                .child(body)
                .into_any_element(),
            None => body,
        };
        let shielded = should_defer && !nested;
        let chrome = snapped(theme.metrics.title_bar, window);
        let mut overrides = overrides;
        let width = overrides.size.width.take();
        let ceiling = overrides.max_size.height.take();
        let right = overrides.inset.left.is_none() && overrides.inset.right.is_some();
        let corner = match right {
            true => Anchor::TopRight,
            false => Anchor::TopLeft,
        };
        let perch = trigger
            .as_ref()
            .and_then(|trigger| trigger.bounds())
            .map(|bounds| {
                let x = match right {
                    true => bounds.right(),
                    false => bounds.left(),
                };
                let position = point(x, bounds.top() - TRIGGER_GAP);
                let offset = point(Pixels::ZERO, bounds.size.height + TRIGGER_GAP + TRIGGER_GAP);
                (position, offset)
            });
        let panel_looks = div()
            .on_children_prepainted({
                let guard = hover_guard.clone();
                move |bounds, _, _| {
                    let Some(bounds) = bounds.into_iter().reduce(|one, other| one.union(&other))
                    else {
                        return;
                    };
                    if let Some(guard) = guard.as_ref() {
                        guard.measure_panel(bounds);
                    }
                    for guard in &bounds_guards {
                        guard.measure_menu(bounds);
                    }
                }
            })
            .id("menu-panel")
            .flex()
            .flex_col()
            .p_1()
            .rounded(theme.radius)
            .border_1()
            .gap_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .key_context(MENU_CONTEXT)
            .when_some(width, |this, width| this.w(width))
            .when_some(ceiling, |this, ceiling| this.max_h(ceiling))
            .when_some(hover_guard, |this, guard| {
                this.on_hover(move |hovered, window, cx| {
                    guard.near(Near::Panel, *hovered, window.window_handle(), cx)
                })
            })
            .occlude()
            .child(body);
        if let Some(dismiss) = dismiss.clone().filter(|_| shielded) {
            arm(dismiss, cx);
        }

        let rising = panel_looks.rising("menu-rise");
        let surface = match should_defer {
            true => {
                let anchored = anchored().anchor(corner);
                match perch {
                    Some((position, offset)) => anchored.position(position).offset(offset),
                    None => anchored.snap_to_window_with_margin(WINDOW_MARGIN),
                }
                .child(rising)
                .into_any_element()
            }
            false => rising.into_any_element(),
        };

        let mut menu = base
            .absolute()
            .flex()
            .flex_col()
            .when_some(dismiss, |this, dismiss| {
                let blocked = move |position: Point<Pixels>| {
                    let reachable = !shielded || position.y < chrome;
                    panel.contains(position, PANEL_SLACK)
                        || dismiss_guards.iter().any(|guard| guard.contains(position))
                        || reachable
                            && trigger
                                .as_ref()
                                .is_some_and(|trigger| trigger.contains(position, Pixels::ZERO))
                };
                this.on_mouse_down_out(move |event, window, cx| {
                    if !blocked(event.position) {
                        dismiss(&(), window, cx);
                    }
                })
            })
            .when(shielded, |this| {
                let viewport = window.viewport_size();
                this.child(
                    anchored().position(point(Pixels::ZERO, chrome)).child(
                        Shield::new("menu-shield")
                            .w(viewport.width)
                            .h(viewport.height - chrome),
                    ),
                )
            })
            .child(surface);

        menu.style().refine(&overrides);

        if should_defer {
            deferred(menu).with_priority(priority).into_any_element()
        } else {
            menu.into_any_element()
        }
    }
}

fn arm(close: Close, cx: &mut App) {
    if cx.try_global::<Escape>().is_none() {
        let armed: Rc<RefCell<Option<Close>>> = Rc::default();
        let watched = armed.clone();
        cx.observe_keystrokes(move |event, window, cx| {
            if event.keystroke.key != ESCAPE_KEY {
                return;
            }
            let Some(close) = watched.borrow_mut().take() else {
                return;
            };
            close(&(), window, cx);
        })
        .detach();

        cx.set_global(Escape(armed));
    }

    let armed = cx.global::<Escape>().0.clone();
    *armed.borrow_mut() = Some(close);
}

/// Picks the side a submenu of `width` opens on: the right when it fits, else the left when
/// that fits, else whichever side has more room and lets it overlap the menu.
fn flips(menu: Bounds<Pixels>, width: Pixels, viewport_width: Pixels) -> bool {
    let right = viewport_width - menu.right() - WINDOW_MARGIN;
    let left = menu.left() - WINDOW_MARGIN;
    match (right >= width, left >= width) {
        (true, _) => false,
        (false, true) => true,
        (false, false) => left > right,
    }
}

fn grown(bounds: Bounds<Pixels>, x: Pixels, y: Pixels) -> Bounds<Pixels> {
    Bounds {
        origin: Point {
            x: bounds.origin.x - x,
            y: bounds.origin.y - y,
        },
        size: Size {
            width: bounds.size.width + x * 2.,
            height: bounds.size.height + y * 2.,
        },
    }
}
