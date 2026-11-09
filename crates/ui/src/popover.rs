//! Triggers that open a [`Menu`]: a button or any face that toggles a menu under it, one of a
//! group of which only one is open at a time, and a context menu opened at the pointer.

use std::rc::Rc;

use rune::{Cursor, El, PointerPress, Tx, el, row};

use crate::button::Button;
use crate::menu::{Menu, TRIGGER_GAP};
use crate::overlay::Placement;
use crate::theme::Theme;

/// How far a context menu sits from the pointer. Opening it right under the cursor would put
/// an item under the button still held.
const NUDGE: f32 = 4.;

/// Finds a group of popovers in the window's state.
type Group<S> = Rc<dyn for<'a> Fn(&'a mut S) -> &'a mut Popovers>;
type Opened<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type Close<S> = Rc<dyn Fn(&mut S)>;

/// Which popover of a group is open, kept in the window's state. Opening one closes the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Popovers {
    open: Option<String>,
}

impl Popovers {
    pub fn close(&mut self) {
        self.open = None;
    }

    /// Whether any popover of the group is open.
    pub fn any(&self) -> bool {
        self.open.is_some()
    }

    pub fn shows(&self, key: &str) -> bool {
        self.open.as_deref() == Some(key)
    }

    /// Opens the popover `key` names and closes any other of the group, for a trigger drawn
    /// apart from the popover itself.
    pub fn open(&mut self, key: &str) {
        self.open = Some(key.to_owned());
    }

    pub fn toggle(&mut self, key: &str) {
        self.open = match self.shows(key) {
            true => None,
            false => Some(key.to_owned()),
        };
    }
}

/// A trigger and the menu it opens. The trigger is a [`Button`] (read as expanded or collapsed)
/// or a `face` drawn as given; a press on it toggles the menu, which hangs under it.
pub struct Popover<S> {
    key: String,
    open: bool,
    group: Group<S>,
    button: Option<Button>,
    face: Option<El>,
    /// What a screen reader calls the `face` trigger.
    name: Option<String>,
    menu: Option<Menu<S>>,
    selected: bool,
    commands: bool,
    placement: Placement,
    on_open: Option<Opened<S>>,
}

impl<S: 'static> Popover<S> {
    /// The popover `key` of `group`, which `access` finds in the window's state.
    pub fn new(
        key: impl Into<String>,
        group: &Popovers,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut Popovers + 'static,
    ) -> Self {
        let key = key.into();
        Self {
            open: group.shows(&key),
            key,
            group: Rc::new(access),
            button: None,
            face: None,
            name: None,
            menu: None,
            selected: false,
            commands: false,
            placement: Placement::BottomStart,
            on_open: None,
        }
    }

    pub fn button(mut self, button: Button) -> Self {
        self.button = Some(button);
        self
    }

    /// Opens the menu from `face`, drawn as given, rather than from a button.
    pub fn face(mut self, face: El) -> Self {
        self.face = Some(face);
        self
    }

    /// Names the `face` trigger for screen readers. A button trigger names itself.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn menu(mut self, menu: Menu<S>) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Paints the button trigger as selected, as it is while the menu is open.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Closes the menu once one of its rows is picked, for a menu of commands. A menu of
    /// toggles leaves it out and stays open until dismissed.
    pub fn commands(mut self) -> Self {
        self.commands = true;
        self
    }

    /// Where the menu hangs against the trigger; under its start edge unless set.
    pub fn placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }

    /// Runs when a press on the trigger opens the menu, not when it closes it, as for reading
    /// afresh what the menu lists.
    pub fn on_open(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.on_open = Some(Rc::new(f));
        self
    }

    /// The trigger, with the open menu pushed into `layer`.
    pub fn build(self, theme: &Theme, layer: &mut Vec<El>) -> El {
        let anchor = format!("{}-trigger", self.key);
        let toggle = {
            let group = self.group.clone();
            let key = self.key.clone();
            let on_open = self.on_open.clone();
            move |s: &mut S, tx: &mut Tx| {
                group(s).toggle(&key);
                if group(s).shows(&key)
                    && let Some(on_open) = &on_open
                {
                    on_open(s, tx);
                }
            }
        };
        let trigger = match (self.face, self.button) {
            (Some(face), _) => {
                let mut face = row()
                    .key(self.key.clone())
                    .role("button")
                    .expanded(self.open)
                    .focusable(true)
                    .cursor(Cursor::Pointer)
                    .on_press_tx(toggle)
                    .child(face);
                if let Some(name) = self.name {
                    face = face.label(name);
                }
                Some(face)
            }
            (None, Some(button)) => Some(
                button
                    .selected(self.selected || self.open)
                    .expanded(self.open)
                    .on_press_tx(toggle)
                    .build(theme),
            ),
            (None, None) => None,
        };
        let head = row().key(anchor.clone()).children(trigger);
        if !self.open {
            return head;
        }
        if let Some(menu) = self.menu {
            let close = self.group.clone();
            let picked = self.group.clone();
            let mut menu = menu
                .anchor(anchor, self.placement, TRIGGER_GAP)
                .on_dismiss(move |s: &mut S| close(s).close());
            if self.commands {
                menu = menu.on_pick(move |s: &mut S, _: &mut Tx| picked(s).close());
            }
            menu.build(theme, layer);
        }
        head
    }
}

/// A menu opened at a point, as a context menu opens where the secondary press landed. Keep
/// the press (from `El::on_secondary_press`) in state while the menu shows and clear it in
/// `on_close`.
pub struct Popup<S> {
    key: String,
    at: (f32, f32),
    menu: Menu<S>,
    close: Option<Close<S>>,
    above: bool,
    sticky: bool,
}

impl<S: 'static> Popup<S> {
    pub fn new(key: impl Into<String>, at: impl Into<Point>, menu: Menu<S>) -> Self {
        Self {
            key: key.into(),
            at: at.into().0,
            menu,
            close: None,
            above: false,
            sticky: false,
        }
    }

    /// Opens the panel upward, ending at the point rather than starting there, for a popup that
    /// belongs to something along the bottom of the window.
    pub fn above(mut self) -> Self {
        self.above = true;
        self
    }

    /// Keeps the panel open after an item is picked, for a menu of toggles the listener may flip
    /// several of in a row. A press outside still closes it.
    pub fn sticky(mut self) -> Self {
        self.sticky = true;
        self
    }

    /// Runs when the popup closes: a press outside, Escape, or a picked row unless `sticky`.
    pub fn on_close(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.close = Some(Rc::new(f));
        self
    }

    pub fn build(self, theme: &Theme, layer: &mut Vec<El>) {
        let point = format!("{}-point", self.key);
        let (x, y) = self.at;
        let placement = match self.above {
            true => Placement::TopStart,
            false => Placement::BottomStart,
        };
        layer.push(
            el().key(point.clone())
                .absolute(x + NUDGE, y)
                .size(0., 0.)
                .a11y_hidden(),
        );
        let mut menu = self.menu.anchor(point, placement, NUDGE);
        if let Some(close) = self.close {
            let picked = close.clone();
            menu = menu.on_dismiss(move |s: &mut S| close(s));
            if !self.sticky {
                menu = menu.on_pick(move |s: &mut S, _: &mut Tx| picked(s));
            }
        }
        menu.build(theme, layer);
    }
}

/// A point in window points, from a pair or from the press a context menu opens at.
pub struct Point((f32, f32));

impl From<(f32, f32)> for Point {
    fn from(at: (f32, f32)) -> Self {
        Self(at)
    }
}

impl From<PointerPress> for Point {
    fn from(press: PointerPress) -> Self {
        Self((press.x, press.y))
    }
}
