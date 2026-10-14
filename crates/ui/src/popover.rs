use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, ClickEvent, Div, StyleRefinement, Window, div};

use crate::button::Button;
use crate::menu::{Menu, Trigger};

/// Runs when a click on the trigger opens the popover.
pub(crate) type Open = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(Clone, Default)]
pub struct Popovers {
    open: Rc<Cell<Option<&'static str>>>,
    triggers: Rc<RefCell<HashMap<&'static str, Trigger>>>,
}

impl Popovers {
    pub fn close(&self) {
        self.open.set(None);
    }

    pub fn shows(&self, key: &'static str) -> bool {
        self.open.get() == Some(key)
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
            menu,
            selected,
            commands,
            on_open,
        } = self;
        let overrides = std::mem::take(base.style());

        let open = group.shows(key);
        let trigger = group.trigger(key);
        let head = button.map(|button| {
            let group = group.clone();
            let observed = trigger.clone();

            head(observed).child(button.selected(selected || open).on_click(
                move |event, window, cx| {
                    group.toggle(key);
                    if let Some(on_open) = on_open.as_ref().filter(|_| group.shows(key)) {
                        on_open(event, window, cx);
                    }
                    cx.refresh_windows();
                },
            ))
        });
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
