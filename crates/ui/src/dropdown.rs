use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Div, ElementId, Pixels, Stateful, StyleRefinement, Window, deferred,
    div, px,
};

use crate::glass::{GLASS_BLUR, blurring, popover_fill};
use crate::motion::{Fading as _, Rising as _};
use crate::theme::ActiveTheme as _;

/// How far below its anchor the panel hangs.
const GAP: Pixels = px(6.);
/// Paints the panel over the page under it but under menus and popups opened from it.
const PRIORITY: usize = 1;

type Close = Box<dyn Fn(&mut Window, &mut App) + 'static>;

/// A field or control with a panel that hangs under it while `open`, as wide as it, floating
/// over the content below in the look of a context menu. The panel takes no focus, so a field
/// in the anchor keeps the keyboard. It comes in with a context menu's entrance each time it
/// opens. A press anywhere outside both the anchor and the panel reaches `on_close`, and
/// whether the panel shows is left to the caller.
#[derive(IntoElement)]
pub struct Dropdown {
    base: Stateful<Div>,
    anchor: AnyElement,
    open: bool,
    children: Vec<AnyElement>,
    close: Option<Close>,
}

impl Dropdown {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, anchor: impl IntoElement) -> Self {
        Self {
            base: div().id(id),
            anchor: anchor.into_any_element(),
            open: false,
            children: Vec::new(),
            close: None,
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn on_close(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.close = Some(Box::new(handler));
        self
    }
}

impl Styled for Dropdown {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for Dropdown {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Dropdown {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let Self {
            base,
            anchor,
            open,
            children,
            close,
        } = self;
        // The panel hangs outside the anchor's box, so a press on it would count as outside.
        // Its bounds from the last prepaint tell the two apart.
        let panel_at: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
        let measured = panel_at.clone();
        let panel = div()
            .id("dropdown-panel")
            .flex()
            .flex_col()
            .p_1()
            .gap_px()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(popover_fill(cx))
            .text_color(theme.popover_foreground)
            .shadow_md()
            .occlude()
            .children(children)
            .rising("dropdown-rise");
        // The entrance is a paint filter, which drops a backdrop inside it, so the frost is a
        // sibling under the panel fading in on the same curve, as a context menu does it.
        let frosted = div()
            .relative()
            .when(blurring(cx), |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(theme.radius)
                        .backdrop_blur(GLASS_BLUR)
                        .fading("dropdown-frost"),
                )
            })
            .child(panel);

        base.relative()
            .child(anchor)
            .when(open, |this| {
                this.child(
                    deferred(
                        div()
                            .absolute()
                            .top_full()
                            .left_0()
                            .right_0()
                            .mt(GAP)
                            .on_children_prepainted(move |bounds, _, _| {
                                measured.set(bounds.first().copied());
                            })
                            .child(frosted),
                    )
                    .with_priority(PRIORITY),
                )
            })
            .when_some(close.filter(|_| open), |this, close| {
                this.on_mouse_down_out(move |event, window, cx| {
                    let inside = panel_at
                        .get()
                        .is_some_and(|bounds| bounds.contains(&event.position));
                    if !inside {
                        close(window, cx);
                    }
                })
            })
    }
}
