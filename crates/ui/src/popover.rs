use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Div, MouseButton, SharedString, StyleRefinement, Window, div,
};

use crate::button::Button;
use crate::menu::{Menu, Trigger};

/// Runs when a click on the trigger opens the popover.
pub(crate) type Open = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
/// Opens or closes the popover from a click on its trigger, shared by either kind of trigger.
type Toggle = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone, Default)]
pub struct Popovers {
    open: Rc<Cell<Option<&'static str>>>,
    triggers: Rc<RefCell<HashMap<&'static str, Trigger>>>,
}

impl Popovers {
    pub fn close(&self) {
        self.open.set(None);
    }

    /// Whether any popover of the group is open.
    pub fn any(&self) -> bool {
        self.open.get().is_some()
    }

    pub fn shows(&self, key: &'static str) -> bool {
        self.open.get() == Some(key)
    }

    /// Opens the popover `key` names and closes any other of the group, for a trigger drawn
    /// apart from the popover itself.
    pub fn open(&self, key: &'static str) {
        self.open.set(Some(key));
    }

    fn toggle(&self, key: &'static str) {
        let next = match self.shows(key) {
            true => None,
            false => Some(key),
        };
        self.open.set(next);
    }

    fn trigger(&self, key: &'static str) -> Trigger {
        self.triggers.borrow_mut().entry(key).or_default().clone()
    }
}

#[derive(IntoElement)]
pub struct Popover {
    base: Div,
    key: &'static str,
    group: Popovers,
    button: Option<Button>,
    /// A trigger the caller draws instead of a button, which the popover makes clickable.
    face: Option<AnyElement>,
    /// What a screen reader calls the `face` trigger, which draws no text the reader can reach.
    name: Option<SharedString>,
    menu: Option<Menu>,
    selected: bool,
    commands: bool,
    on_open: Option<Open>,
}

impl Popover {
    pub fn new(key: &'static str, group: Popovers) -> Self {
        Self {
            base: div(),
            key,
            group,
            button: None,
            face: None,
            name: None,
            menu: None,
            selected: false,
            commands: false,
            on_open: None,
        }
    }

    pub fn button(mut self, button: Button) -> Self {
        self.button = Some(button);
        self
    }

    /// Opens the popover from `face`, drawn as given, rather than from a button. A click on it
    /// toggles the popover and goes no further.
    pub fn face(mut self, face: impl IntoElement) -> Self {
        self.face = Some(face.into_any_element());
        self
    }

    /// Names the `face` trigger for screen readers. A button trigger names itself and ignores it.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn menu(mut self, menu: Menu) -> Self {
        self.menu = Some(menu);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn commands(mut self) -> Self {
        self.commands = true;
        self
    }

    /// Runs `handler` when a click on the trigger opens the popover, not when it closes it, as
    /// for reading afresh what the menu lists.
    pub fn on_open(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open = Some(Box::new(handler));
        self
    }
}

impl Styled for Popover {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Popover {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            key,
            group,
            button,
            face,
            name,
            menu,
            selected,
            commands,
            on_open,
        } = self;
        let overrides = std::mem::take(base.style());

        let open = group.shows(key);
        let trigger = group.trigger(key);
        let toggle: Toggle = {
            let group = group.clone();
            Rc::new(move |event, window, cx| {
                group.toggle(key);
                if let Some(on_open) = on_open.as_ref().filter(|_| group.shows(key)) {
                    on_open(event, window, cx);
                }
                cx.refresh_windows();
            })
        };
        let head = match (face, button) {
            (Some(face), _) => Some(
                head(trigger.clone()).max_w_full().child(
                    div()
                        .id(key)
                        .role(gpui::accesskit::Role::Button)
                        .aria_expanded(open)
                        .when_some(name, |this, name| this.aria_label(name))
                        .max_w_full()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |event, window, cx| toggle(event, window, cx))
                        .child(face),
                ),
            ),
            (None, Some(button)) => Some(
                head(trigger.clone()).child(
                    button
                        .selected(selected || open)
                        .expanded(open)
                        .on_click(move |event, window, cx| toggle(event, window, cx)),
                ),
            ),
            (None, None) => None,
        };
        let body = menu.filter(|_| open).map(|menu| {
            let outside = group.clone();
            let chosen = group.clone();

            menu.trigger(trigger)
                .on_dismiss(move |_, _, cx| {
                    outside.close();
                    cx.refresh_windows();
                })
                .when(commands, |menu| {
                    menu.on_action(move |_, _, cx| {
                        chosen.close();
                        cx.refresh_windows();
                    })
                })
        });

        let mut popover = base.relative().children(head).children(body);
        popover.style().refine(&overrides);
        popover
    }
}

fn head(trigger: Trigger) -> Div {
    div().on_children_prepainted(move |bounds, _, _| trigger.observe(bounds))
}
