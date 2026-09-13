use gpui::prelude::*;
use gpui::{App, Hsla, Pixels, SharedString, StyleRefinement, Window, px};

use crate::button::Button;
use crate::menu::{Menu, MenuItem, TRIGGER_GAP};
use crate::popover::{Popover, Popovers};
use crate::theme::ActiveTheme as _;

enum Face {
    Label(SharedString),
    Plain(SharedString),
    Icon(&'static str),
}

#[derive(IntoElement)]
pub struct Picker {
    style: StyleRefinement,
    key: &'static str,
    group: Popovers,
    face: Face,
    tooltip: Option<&'static str>,
    above: bool,
    tint: Option<Hsla>,
    selected: bool,
    small: bool,
    width: Pixels,
    left: bool,
    sticky: bool,
    items: Vec<MenuItem>,
    menu: Option<Menu>,
}

impl Picker {
    pub const NARROW: Pixels = px(170.);
    pub const REGULAR: Pixels = px(190.);
    pub const WIDE: Pixels = px(260.);

    pub fn new(key: &'static str, group: &Popovers, current: impl Into<SharedString>) -> Self {
        Self {
            style: StyleRefinement::default(),
            key,
            group: group.clone(),
            face: Face::Label(current.into()),
            tooltip: None,
            above: false,
            tint: None,
            selected: false,
            small: true,
            width: Self::REGULAR,
            left: false,
            sticky: false,
            items: Vec::new(),
            menu: None,
        }
    }

    pub fn icon(key: &'static str, group: &Popovers, icon: &'static str) -> Self {
        Self {
            face: Face::Icon(icon),
            ..Self::new(key, group, "")
        }
    }

    /// A labelled trigger without the chevron, for a popover that holds a control rather than a
    /// list of choices.
    pub fn plain(key: &'static str, group: &Popovers, label: impl Into<SharedString>) -> Self {
        Self {
            face: Face::Plain(label.into()),
            ..Self::new(key, group, "")
        }
    }

    pub fn tooltip(mut self, key: &'static str) -> Self {
        self.tooltip = Some(key);
        self
    }

    pub fn tooltip_above(mut self, key: &'static str) -> Self {
        self.tooltip = Some(key);
        self.above = true;
        self
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn large(mut self) -> Self {
        self.small = false;
        self
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    pub fn left(mut self) -> Self {
        self.left = true;
        self
    }

    pub fn sticky(mut self) -> Self {
        self.sticky = true;
        self
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = MenuItem>) -> Self {
        self.items.extend(items);
        self
    }

    pub fn menu(mut self, menu: Menu) -> Self {
        self.menu = Some(menu);
        self
    }
}

impl Styled for Picker {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Picker {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            style,
            key,
            group,
            face,
            tooltip,
            above,
            tint,
            selected,
            small,
            width,
            left,
            sticky,
            items,
            menu,
        } = self;

        let theme = *cx.theme();
        let drop = match small {
            true => theme.metrics.control_small,
            false => theme.metrics.control,
        } + TRIGGER_GAP;

        let button = match face {
            Face::Label(current) => Button::new(SharedString::from(format!("{key}-picker")))
                .label(current)
                .trailing("icons/chevron-down.svg")
                .outline(),
            Face::Plain(label) => Button::new(SharedString::from(format!("{key}-picker")))
                .label(label)
                .outline(),
            Face::Icon(icon) => Button::new(SharedString::from(format!("{key}-picker")))
                .icon(icon)
                .ghost(),
        };
        let button = button
            .when(small, Button::small)
            .when_some(tooltip, |button, key| match above {
                true => button.tooltip_above(key),
                false => button.tooltip(key),
            })
            .when_some(tint, Button::tint);

        let menu = menu
            .unwrap_or_else(|| Menu::new(SharedString::from(format!("{key}-menu"))).w(width))
            .items(items)
            .top(drop)
            .when_else(left, |menu| menu.left_0(), |menu| menu.right_0());

        let mut popover = Popover::new(key, group)
            .button(button)
            .selected(selected)
            .menu(menu)
            .when(!sticky, Popover::commands);
        *popover.style() = style;
        popover
    }
}
