use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, FontWeight, Global, MouseButton, Pixels,
    Point, ScrollWheelEvent, SharedString, StyleRefinement, Subscription, Window, WindowId,
    anchored, deferred, div, point, px,
};

use crate::button::Button;
use crate::linger::Linger;
use crate::menu::{ESCAPE_KEY, menu_open};
use crate::metrics::{Rounding, Text, snapped};
use crate::motion::Rising as _;
use crate::scrollbar::Scrollbar;
use crate::scroller::middle_scroll;
use crate::shield::Shield;
use crate::theme::ActiveTheme as _;

/// How many covers wide a dialog is.
const WIDTH: f32 = 2.75;
/// How far the close button sits from the top and the end of the dialog. Its cross then sits
/// as far in as a small button's padding takes it.
const CLOSE_INSET: Pixels = px(11.);
/// How much of the close button shows until the pointer comes to it.
const CLOSE_FADE: f32 = 0.7;
/// How many times the theme radius a dialog's corners take. A surface this large reads square at
/// the radius a button uses, and scaling it keeps a dialog square when corners are set to square.
/// The scaled radius never passes what Rounded gives, since Round doubled bulges a dialog.
const ROUNDING: f32 = 2.;

type Dismiss = Rc<dyn Fn(&(), &mut Window, &mut App)>;

thread_local! {
    /// The mark of the dialog rendered last. Dialogs render in the order they stack, so the one
    /// holding this mark is the topmost.
    static LATEST: Cell<u64> = const { Cell::new(0) };
}

/// Closes a dialog on escape, wherever the key focus is. It lives in the dialog's element state,
/// so it goes away with the dialog. Only the topmost dialog answers, and an open menu over it
/// takes the escape first.
struct EscapeGuard {
    mark: Rc<Cell<u64>>,
    dismiss: Rc<RefCell<Option<Dismiss>>>,
    _keys: Subscription,
}

impl EscapeGuard {
    fn new(window: WindowId, cx: &mut Context<Self>) -> Self {
        let mark = Rc::new(Cell::new(0));
        let dismiss: Rc<RefCell<Option<Dismiss>>> = Rc::default();
        let keys = cx.intercept_keystrokes({
            let (mark, dismiss) = (mark.clone(), dismiss.clone());
            move |event, target, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key != ESCAPE_KEY
                    || keystroke.modifiers.modified()
                    || target.window_handle().window_id() != window
                    || LATEST.with(Cell::get) != mark.get()
                    || menu_open(cx)
                {
                    return;
                }
                let Some(dismiss) = dismiss.borrow().clone() else {
                    return;
                };
                cx.stop_propagation();
                dismiss(&(), target, cx);
            }
        });
        Self {
            mark,
            dismiss,
            _keys: keys,
        }
    }
}

#[derive(Default)]
struct Bars(HashMap<ElementId, Entity<Scrollbar>>);

impl Global for Bars {}

fn bar(id: &ElementId, cx: &mut App) -> Entity<Scrollbar> {
    if let Some(known) = cx.try_global::<Bars>().and_then(|bars| bars.0.get(id)) {
        return known.clone();
    }
    let bar = cx.new(|_| Scrollbar::inset());
    cx.default_global::<Bars>()
        .0
        .insert(id.clone(), bar.clone());
    bar
}

#[derive(IntoElement)]
pub struct Modal {
    base: Div,
    id: ElementId,
    title: SharedString,
    detail: Option<SharedString>,
    body: Vec<AnyElement>,
    actions: Vec<AnyElement>,
    /// What sits at the end of the title row, before the close button.
    trailing: Option<AnyElement>,
    dismiss: Option<Dismiss>,
    close_button: bool,
}

impl Modal {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            id: id.into(),
            title: title.into(),
            detail: None,
            body: Vec::new(),
            actions: Vec::new(),
            trailing: None,
            dismiss: None,
            close_button: false,
        }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn child(mut self, body: impl IntoElement) -> Self {
        self.body.push(body.into_any_element());
        self
    }

    /// Adds an answer to the row along the bottom, which stands as tall as a text field and
    /// outlines a ghost button.
    pub fn action(mut self, action: Button) -> Self {
        self.actions.push(action.answer().into_any_element());
        self
    }

    /// Puts an element at the end of the title row, for a side door out of the dialog that
    /// does not belong among its answers.
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }

    /// Draws a cross in the top corner, which dismisses the modal the same way clicking
    /// outside it does. For a modal the user is meant to be able to walk away from without
    /// reading the buttons.
    pub fn close_button(mut self) -> Self {
        self.close_button = true;
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.dismiss = Some(Rc::new(handler));
        self
    }

    /// Sends the body of an open modal back to the top, for a caller that swapped its content.
    pub fn rewind(id: impl Into<ElementId>, cx: &mut App) {
        let id = id.into();
        let Some(bar) = cx
            .try_global::<Bars>()
            .and_then(|bars| bars.0.get(&id).cloned())
        else {
            return;
        };
        bar.read(cx).scroll().set_offset(Point::default());
    }
}

impl Styled for Modal {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Modal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let room = theme.metrics.inset;
        let gap = theme.metrics.pad * 2.;
        let heading = theme.text(Text::Heading);
        let Self {
            mut base,
            id,
            title,
            detail,
            body,
            actions,
            trailing,
            dismiss,
            close_button,
        } = self;
        let outside = dismiss.clone();
        let mark = LATEST.with(|latest| {
            latest.set(latest.get() + 1);
            latest.get()
        });
        if let Some(dismiss) = dismiss.clone() {
            let window_id = window.window_handle().window_id();
            let guard = window.use_keyed_state(
                ElementId::NamedChild(Arc::new(id.clone()), "escape".into()),
                cx,
                |_, cx| EscapeGuard::new(window_id, cx),
            );
            let guard = guard.read(cx);
            guard.mark.set(mark);
            *guard.dismiss.borrow_mut() = Some(dismiss);
        }
        // with no action row under it the body carries the bottom padding itself, so the
        // panel is inset by the same amount all the way round. Above an action row it keeps
        // enough of it that the focus ring of a field at its foot is not cut off by the
        // scrolling body, and the row takes the rest.
        let tail = match actions.is_empty() {
            true => room,
            false => theme.metrics.pad,
        };
        let close = dismiss.clone().filter(|_| close_button);
        let overrides = std::mem::take(base.style());
        let scroller = bar(&id, cx);
        scroller.read(cx).sync();
        let body_id = SharedString::from(format!("modal-body-{id:?}"));
        // The panel carries its own node named for the title, since nothing
        // else in the frame has both an id and a role.
        let name = title.clone();
        let about = detail.clone();

        // The window is the modal's frame, not the pane that raised it: anchored to window
        // coordinates and deferred, so a page with chrome floating over it cannot cover the
        // modal or crop it. The scrim dims the title bar with the rest of the window, and the
        // panel centres in the room under the bar.
        let chrome = snapped(theme.metrics.title_bar, window);
        let viewport = window.viewport_size();
        let frame = div()
            .w(viewport.width)
            .h(viewport.height)
            .p(theme.metrics.inset)
            .pt(chrome + theme.metrics.inset)
            .flex()
            .items_center()
            .justify_center()
            // The scrim and the panel leave separately. The panel shrinks a little as it fades,
            // and a scrim shrinking with it would bare the window's edges.
            .child(
                Linger::new(
                    ElementId::NamedChild(Arc::new(id.clone()), "scrim".into()),
                    Shield::new(id.clone())
                        .absolute()
                        .inset_0()
                        .bg(theme.overlay)
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(outside) = &outside {
                                outside(&(), window, cx);
                            }
                        }),
                )
                .fading(),
            )
            .child({
                let leaving = ElementId::NamedChild(Arc::new(id.clone()), "leave".into());
                let mut panel = base
                    .id((id, "panel"))
                    .role(gpui::accesskit::Role::Dialog)
                    .aria_label(name)
                    .when_some(about, |this, detail| this.aria_description(detail))
                    .relative()
                    .occlude()
                    .w(theme.metrics.cover * WIDTH)
                    .max_w_full()
                    .max_h_full()
                    .flex()
                    .flex_col()
                    .rounded(corners(theme.radius))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_start()
                            .gap_2()
                            .pl(room)
                            .pt(room)
                            // the title keeps clear of the close button in the corner
                            .pr(match close.is_some() {
                                true => CLOSE_INSET + theme.metrics.control_small + gap,
                                false => room,
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_1()
                                    .min_w_0()
                                    .flex_col()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .text_size(heading)
                                            .line_height(heading)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .when_some(detail, |this, detail| {
                                        this.child(
                                            div()
                                                .text_size(theme.text(Text::Body))
                                                .text_color(theme.muted_foreground)
                                                .child(detail),
                                        )
                                    }),
                            )
                            .children(trailing),
                    )
                    .children(close.map(|close| {
                        div()
                            .absolute()
                            .top(CLOSE_INSET)
                            .right(CLOSE_INSET)
                            .opacity(CLOSE_FADE)
                            .hover(|this| this.opacity(1.))
                            .child(
                                Button::new("modal-close")
                                    .icon("icons/x.svg")
                                    .small()
                                    .ghost()
                                    .hoverless()
                                    .tooltip("common-dismiss")
                                    .on_click(move |_, window, cx| close(&(), window, cx)),
                            )
                    }))
                    .when(!body.is_empty(), |this| {
                        this.child(
                            div()
                                .relative()
                                .flex()
                                .flex_1()
                                .w_full()
                                .min_h_0()
                                .overflow_hidden()
                                .child(
                                    middle_scroll(div(), &scroller)
                                        .id(body_id.clone())
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .w_full()
                                        .min_w_0()
                                        .min_h_0()
                                        .gap(gap)
                                        .px(room)
                                        .pt(room)
                                        .pb(tail)
                                        .overflow_y_scroll()
                                        .track_scroll(scroller.read(cx).scroll())
                                        .on_scroll_wheel({
                                            let gliding = scroller.clone();
                                            move |event: &ScrollWheelEvent, window, cx| {
                                                if event.delta.precise() {
                                                    return;
                                                }
                                                gliding.update(cx, |bar, _| bar.nudge(window));
                                            }
                                        })
                                        .children(
                                            body.into_iter()
                                                .map(|child| div().flex_none().child(child)),
                                        ),
                                )
                                .child(scroller.clone()),
                        )
                    })
                    .when(!actions.is_empty(), |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_none()
                                .justify_end()
                                .gap_2()
                                .px(room)
                                .pb(room)
                                .pt(room - tail)
                                .children(actions),
                        )
                    });
                panel.style().refine(&overrides);
                Linger::new(leaving, panel.rising("modal-rise"))
            });

        deferred(
            anchored()
                .position(point(Pixels::ZERO, Pixels::ZERO))
                .child(frame),
        )
    }
}

/// The corner radius of a large floating surface for a theme radius: `ROUNDING` times it, capped
/// at what the Rounded setting gives. Dialogs, toasts and notices share it, so they round alike.
pub fn corners(radius: Pixels) -> Pixels {
    (radius * ROUNDING).min(Rounding::Rounded.radius() * ROUNDING)
}
