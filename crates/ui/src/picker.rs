//! A button that shows the current choice of a setting and opens the menu of its choices.

use rune::{El, Tx};

use crate::button::Button;
use crate::color::Hsla;
use crate::menu::{Menu, MenuItem};
use crate::overlay::Placement;
use crate::popover::{Popover, Popovers};
use crate::theme::Theme;

enum Face {
    Label(String),
    Plain(String),
    Icon(String),
}

/// A picker: an outlined small button reading the current choice with a chevron, or an icon
/// button, opening a [`Menu`] of its choices as one popover of a group.
pub struct Picker<S> {
    popover: Popover<S>,
    key: String,
    face: Face,
    name: Option<String>,
    leading: Option<El>,
    tooltip: Option<(String, bool)>,
    tint: Option<Hsla>,
    status: Option<Hsla>,
    selected: bool,
    small: bool,
    ghost: bool,
    width: f32,
    left: bool,
    sticky: bool,
    disabled: bool,
    items: Vec<MenuItem<S>>,
    menu: Option<Menu<S>>,
    scroll_after: Option<f32>,
}

impl<S: 'static> Picker<S> {
    pub const NARROW: f32 = 170.;
    pub const REGULAR: f32 = 190.;
    pub const WIDE: f32 = 260.;

    /// A picker reading `current`, the popover `key` of `group`, which `access` finds in the
    /// window's state.
    pub fn new(
        key: impl Into<String>,
        group: &Popovers,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut Popovers + 'static,
        current: impl Into<String>,
    ) -> Self {
        let key = key.into();
        Self {
            popover: Popover::new(key.clone(), group, access),
            key,
            face: Face::Label(current.into()),
            name: None,
            leading: None,
            tooltip: None,
            tint: None,
            status: None,
            selected: false,
            small: true,
            ghost: false,
            width: Self::REGULAR,
            left: false,
            sticky: false,
            disabled: false,
            items: Vec::new(),
            menu: None,
            scroll_after: None,
        }
    }

    /// A picker whose trigger is the icon `icon` rather than the current choice.
    pub fn icon(
        key: impl Into<String>,
        group: &Popovers,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut Popovers + 'static,
        icon: impl Into<String>,
    ) -> Self {
        Self {
            face: Face::Icon(icon.into()),
            ..Self::new(key, group, access, "")
        }
    }

    /// A labelled trigger without the chevron, for a popover that holds a control rather than a
    /// list of choices.
    pub fn plain(
        key: impl Into<String>,
        group: &Popovers,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut Popovers + 'static,
        label: impl Into<String>,
    ) -> Self {
        Self {
            face: Face::Plain(label.into()),
            ..Self::new(key, group, access, "")
        }
    }

    /// Names the trigger for screen readers after the setting it changes, so the current choice
    /// is read as its description rather than as its name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Draws `leading` before the current choice on the trigger.
    pub fn leading(mut self, leading: El) -> Self {
        self.leading = Some(leading);
        self
    }

    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some((text.into(), false));
        self
    }

    pub fn tooltip_above(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some((text.into(), true));
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    /// Marks the trigger with a dot of `color` over its icon's top left corner.
    pub fn status(mut self, color: Hsla) -> Self {
        self.status = Some(color);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Drops the outline of a labelled trigger, for one that sits among ghost buttons.
    pub fn ghost(mut self) -> Self {
        self.ghost = true;
        self
    }

    pub fn large(mut self) -> Self {
        self.small = false;
        self
    }

    /// The menu's width, [`Picker::REGULAR`] unless set.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Hangs the menu from the trigger's start edge rather than its end.
    pub fn left(mut self) -> Self {
        self.left = true;
        self
    }

    /// Keeps the menu open after a pick, for a menu of toggles.
    pub fn sticky(mut self) -> Self {
        self.sticky = true;
        self
    }

    /// Holds the trigger still, so the menu cannot open, as while an earlier choice applies.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn item(mut self, item: MenuItem<S>) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = MenuItem<S>>) -> Self {
        self.items.extend(items);
        self
    }

    /// A whole menu in place of the default one, for a menu with a search or a header.
    pub fn menu(mut self, menu: Menu<S>) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Caps the default menu at `height` and scrolls what does not fit.
    pub fn scroll_after(mut self, height: f32) -> Self {
        self.scroll_after = Some(height);
        self
    }

    /// Runs when a press opens the menu, as for reading afresh what it lists.
    pub fn on_open(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.popover = self.popover.on_open(f);
        self
    }

    /// The trigger, with the open menu pushed into `layer`.
    pub fn build(self, theme: &Theme, layer: &mut Vec<El>) -> El {
        let key = format!("{}-picker", self.key);
        let mut button = match self.face {
            Face::Label(current) => Button::new(key)
                .label(current)
                .trailing("icons/chevron-down.svg")
                .outline(),
            Face::Plain(label) => Button::new(key).label(label).outline(),
            Face::Icon(icon) => Button::new(key).icon(icon).ghost(),
        };
        if self.small {
            button = button.small();
        }
        if self.ghost {
            button = button.ghost();
        }
        if let Some(name) = self.name.clone() {
            button = button.name(name);
        }
        if let Some(leading) = self.leading {
            button = button.leading(leading);
        }
        if let Some((tip, above)) = self.tooltip {
            button = match above {
                true => button.tooltip_above(tip),
                false => button.tooltip(tip),
            };
        }
        if let Some(tint) = self.tint {
            button = button.tint(tint);
        }
        if let Some(status) = self.status {
            button = button.status(status);
        }
        let button = button.disabled(self.disabled);
        let mut menu = self.menu.unwrap_or_else(|| {
            let menu = Menu::new(format!("{}-menu", self.key)).width(self.width);
            match self.scroll_after {
                Some(height) => menu.max_height(height),
                None => menu,
            }
        });
        menu = menu.items(self.items);
        if let Some(name) = self.name {
            menu = menu.name(name);
        }
        let mut popover = self
            .popover
            .button(button)
            .selected(self.selected)
            .menu(menu)
            .placement(match self.left {
                true => Placement::BottomStart,
                false => Placement::BottomEnd,
            });
        if !self.sticky {
            popover = popover.commands();
        }
        popover.build(theme, layer)
    }
}
