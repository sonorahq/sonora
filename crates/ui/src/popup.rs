use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, MouseButton, Pixels, Point, StyleRefinement, Window, anchored, point, px};

use crate::menu::{self, Menu};

const MARGIN: Pixels = px(8.);
/// How far the panel sits from the pointer. Opening it right under the cursor puts an item
/// beneath the button that is still down, so letting go without moving would pick it.
const NUDGE: Pixels = px(4.);

type Close = Box<dyn Fn(&(), &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Popup {
    at: Point<Pixels>,
    menu: Menu,
    close: Option<Close>,
    above: bool,
    sticky: bool,
}

impl Popup {
    pub fn new(at: Point<Pixels>, menu: Menu) -> Self {
        Self {
            at,
            menu,
            close: None,
            above: false,
            sticky: false,
        }
    }

    /// Opens the panel upward, ending at `at` rather than starting there, for a popup that
    /// belongs to something along the bottom of the window.
    pub fn above(mut self) -> Self {
        self.above = true;
        self
    }

    /// Keeps the panel open after an item is picked, for a menu of toggles the listener may flip
    /// several of in a row. A click outside still closes it.
    pub fn sticky(mut self) -> Self {
        self.sticky = true;
        self
    }

    pub fn on_close(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.close = Some(Box::new(handler));
        self
    }
}

impl Styled for Popup {
    fn style(&mut self) -> &mut StyleRefinement {
        self.menu.style()
    }
}

impl RenderOnce for Popup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            at,
            menu,
            close,
            above,
            sticky,
        } = self;
        let menu = menu.pressed_at(at);
        let (menu, nudge) = match above {
            true => (menu.above(), point(NUDGE, -NUDGE)),
            false => (menu, point(NUDGE, NUDGE)),
        };
        let menu = match close {
            None => menu,
            Some(close) => {
                let close = Rc::new(close);
                let outside = close.clone();
                let toggled = close.clone();
                let searches = menu.searches();
                let menu = match sticky {
                    true => menu,
                    false => menu.on_action(move |_, window, cx| close(&(), window, cx)),
                };

                menu.on_dismiss(move |_, window, cx| outside(&(), window, cx))
                    .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                        cx.stop_propagation();
                        menu::release(&searches, window, cx);
                        toggled(&(), window, cx);
                    })
            }
        };

        anchored()
            .position(at + nudge)
            .snap_to_window_with_margin(MARGIN)
            .child(menu)
    }
}
