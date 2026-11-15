use std::rc::Rc;
use std::sync::Arc;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, AnyView, App, Div, ElementId, Entity, FocusHandle, Focusable, SharedString,
    StyleRefinement, Window, actions, div,
};

use crate::metrics::Text;
use crate::theme::ActiveTheme as _;

actions!(form, [Submit, NextField, PreviousField]);

pub const FORM_CONTEXT: &str = "Form";

/// What a form does when it is submitted or dismissed.
type Handler = Rc<dyn Fn(&(), &mut Window, &mut App)>;

/// A form: its fields one under another, or side by side in a row, with the keys a form is
/// expected to answer. Enter submits it, Escape dismisses it, and Tab and Shift-Tab move
/// between its fields in the order they were added, wrapping round at either end.
///
/// The keys only reach the form while one of its fields has the focus. A form with no submit
/// or dismiss handler lets that key go on to whatever holds it, such as a dialog that also
/// answers it while none of its fields has the focus.
#[derive(IntoElement)]
pub struct Form {
    base: Div,
    id: ElementId,
    /// What screen readers call the form, when it needs a name of its own.
    name: Option<SharedString>,
    items: Vec<Item>,
    /// Fields the caller lays out among the children, which Tab visits after the form's own.
    placed: Vec<FocusHandle>,
    submit: Option<Handler>,
    dismiss: Option<Handler>,
}

/// One line of a form.
enum Item {
    Field(Box<Field>),
    Row(Vec<Field>),
    Other(AnyElement),
}

impl Form {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div(),
            id: id.into(),
            name: None,
            items: Vec::new(),
            placed: Vec::new(),
            submit: None,
            dismiss: None,
        }
    }

    /// Names the form for screen readers.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Adds a field on a line of its own.
    pub fn field(mut self, field: Field) -> Self {
        self.items.push(Item::Field(Box::new(field)));
        self
    }

    /// Adds every one of `fields`, each on a line of its own.
    pub fn fields(mut self, fields: impl IntoIterator<Item = Field>) -> Self {
        self.items
            .extend(fields.into_iter().map(|field| Item::Field(Box::new(field))));
        self
    }

    /// Adds `fields` side by side on one line, sharing its width. Tab visits them left to right.
    pub fn row(mut self, fields: impl IntoIterator<Item = Field>) -> Self {
        self.items.push(Item::Row(fields.into_iter().collect()));
        self
    }

    /// Adds something that is not a field, such as a note, on a line of its own.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.items.push(Item::Other(child.into_any_element()));
        self
    }

    /// Adds fields the caller lays out itself, inside a child, to the order Tab visits them in,
    /// after the fields the form lays out. For a form whose fields sit in a table or a grid.
    pub fn placed(mut self, fields: impl IntoIterator<Item = FocusHandle>) -> Self {
        self.placed.extend(fields);
        self
    }

    /// Runs `handler` when Enter is pressed in one of the form's fields.
    pub fn on_submit(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.submit = Some(Rc::new(handler));
        self
    }

    /// Runs `handler` when Escape is pressed in one of the form's fields.
    pub fn on_dismiss(mut self, handler: impl Fn(&(), &mut Window, &mut App) + 'static) -> Self {
        self.dismiss = Some(Rc::new(handler));
        self
    }
}

impl Styled for Form {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Form {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            id,
            name,
            items,
            placed,
            submit,
            dismiss,
        } = self;
        let theme = cx.theme();
        let overrides = std::mem::take(base.style());
        let order: Rc<Vec<FocusHandle>> = Rc::new(
            items
                .iter()
                .flat_map(|item| match item {
                    Item::Field(field) => vec![field.focus.clone()],
                    Item::Row(fields) => fields.iter().map(|field| field.focus.clone()).collect(),
                    Item::Other(_) => Vec::new(),
                })
                .chain(placed)
                .collect(),
        );
        let forward = order.clone();
        let gap = theme.metrics.pad;

        let lines = items.into_iter().map(|item| match item {
            Item::Field(field) => (*field).into_any_element(),
            Item::Row(fields) => div()
                .flex()
                .items_start()
                .gap(gap)
                .children(
                    fields
                        .into_iter()
                        .map(|field| div().flex_1().min_w_0().child(field)),
                )
                .into_any_element(),
            Item::Other(element) => element,
        });

        let mut form = base
            .id(id)
            .role(Role::Form)
            .when_some(name, |form, name| form.aria_label(name))
            .key_context(FORM_CONTEXT)
            .flex()
            .flex_col()
            .gap(gap)
            .on_action(move |_: &NextField, window, cx| step(&forward, 1, window, cx))
            .on_action(move |_: &PreviousField, window, cx| step(&order, -1, window, cx))
            .on_action(move |_: &Submit, window, cx| match &submit {
                Some(submit) => {
                    cx.stop_propagation();
                    submit(&(), window, cx);
                }
                None => cx.propagate(),
            })
            .on_action(move |_: &crate::Dismiss, window, cx| match &dismiss {
                Some(dismiss) => {
                    cx.stop_propagation();
                    dismiss(&(), window, cx);
                }
                None => cx.propagate(),
            })
            .children(lines);
        form.style().refine(&overrides);
        form
    }
}

/// One field of a form: its label above the control, an optional element at the end of the
/// label's line, and a muted note or an error under the control. The error replaces the note
/// and is announced to screen readers as it appears.
///
/// A labelled field reaches screen readers as a group named for its label. The control inside
/// still speaks its own name, so a control whose hint says something other than the label takes
/// the label as its name where it is built.
#[derive(IntoElement)]
pub struct Field {
    base: Div,
    control: AnyView,
    focus: FocusHandle,
    label: Option<SharedString>,
    aside: Option<AnyElement>,
    end: Option<AnyElement>,
    note: Option<SharedString>,
    error: Option<SharedString>,
}

impl Field {
    /// A field around `control`, which Tab moves the focus into.
    pub fn new<V: Render + Focusable>(control: &Entity<V>, cx: &App) -> Self {
        Self {
            base: div(),
            control: control.clone().into(),
            focus: control.read(cx).focus_handle(cx),
            label: None,
            aside: None,
            end: None,
            note: None,
            error: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Draws `element` at the end of the label's line, as a button that adds another field.
    pub fn aside(mut self, element: impl IntoElement) -> Self {
        self.aside = Some(element.into_any_element());
        self
    }

    /// Draws `element` after the control on its line, as a button that removes the field.
    pub fn end(mut self, element: impl IntoElement) -> Self {
        self.end = Some(element.into_any_element());
        self
    }

    /// A muted line under the control that says what goes in it.
    pub fn note(mut self, note: impl Into<SharedString>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// What is wrong with the field's value, drawn in the danger colour under the control in
    /// place of the note. `None` shows the note again.
    pub fn error(mut self, error: Option<SharedString>) -> Self {
        self.error = error;
        self
    }
}

impl Styled for Field {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Field {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            control,
            focus,
            label,
            aside,
            end,
            note,
            error,
        } = self;
        let theme = cx.theme();
        let overrides = std::mem::take(base.style());
        let owner = Arc::new(ElementId::from(&focus));
        let small = theme.text(Text::Small);
        let muted = theme.muted_foreground;
        let danger = theme.danger;

        let named = label.clone();
        let whole = ElementId::NamedChild(owner.clone(), "field".into());
        let heading = (label.is_some() || aside.is_some()).then(|| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .children(label.map(|label| {
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(small)
                        .text_color(muted)
                        .child(label)
                }))
                .children(aside)
        });
        let under = match error {
            Some(error) => Some(
                div()
                    .id(ElementId::NamedChild(owner, "field-error".into()))
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .text_size(small)
                    .text_color(danger)
                    .child(error)
                    .into_any_element(),
            ),
            None => note.map(|note| {
                div()
                    .id(ElementId::NamedChild(owner, "field-note".into()))
                    .role(Role::Paragraph)
                    .aria_label(note.clone())
                    .text_size(small)
                    .text_color(muted)
                    .child(note)
                    .into_any_element()
            }),
        };

        let mut field = base
            .id(whole)
            .when_some(named, |field, label| {
                field.role(Role::Group).aria_label(label)
            })
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .children(heading)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(control))
                    .children(end),
            )
            .children(under);
        field.style().refine(&overrides);
        field
    }
}

/// Moves the focus `by` fields along `order` from the one that holds it, wrapping round at either
/// end. With none of them focused it goes to the first, or the last going backwards.
fn step(order: &[FocusHandle], by: isize, window: &mut Window, cx: &mut App) {
    let count = order.len() as isize;
    if count == 0 {
        cx.propagate();
        return;
    }
    let next = match order
        .iter()
        .position(|focus| focus.contains_focused(window, cx))
    {
        Some(at) => (at as isize + by).rem_euclid(count),
        None if by < 0 => count - 1,
        None => 0,
    };
    window.focus(&order[next as usize], cx);
}
