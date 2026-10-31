use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, ElementId, FontWeight, Interactivity, SharedString, StyleRefinement,
    Window, div,
};

use crate::label::eyebrow;
use crate::theme::ActiveTheme as _;

#[derive(IntoElement)]
pub struct InfoCard {
    base: Div,
    title: SharedString,
    /// The id under which the card reaches screen readers, which it does only once given one.
    spoken: Option<ElementId>,
    children: Vec<AnyElement>,
    borderless: bool,
    stretch: bool,
    plain: bool,
    untitled: bool,
    flush: bool,
}

impl InfoCard {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            title: title.into(),
            spoken: None,
            children: Vec::new(),
            borderless: false,
            stretch: false,
            plain: false,
            untitled: false,
            flush: false,
        }
    }

    pub fn borderless(mut self) -> Self {
        self.borderless = true;
        self
    }

    pub fn stretch(mut self) -> Self {
        self.stretch = true;
        self
    }

    /// Lays the title and the content out as the card does, with no frame, fill or inset around
    /// them, for a page that separates its parts by space alone.
    pub fn plain(mut self) -> Self {
        self.plain = true;
        self
    }

    /// Leaves the eyebrow title out, for a card whose heading sits outside it or that needs none.
    pub fn untitled(mut self) -> Self {
        self.untitled = true;
        self
    }

    /// Drops the inset and the gap between children for a thin band above and below, so rows that
    /// pad themselves run edge to edge and read as one surface.
    pub fn flush(mut self) -> Self {
        self.flush = true;
        self
    }

    /// Exposes the card to screen readers as a group named by its title, under the element id
    /// `id`. Without it the title is drawn but never read.
    pub fn spoken(mut self, id: impl Into<ElementId>) -> Self {
        self.spoken = Some(id.into());
        self
    }
}

impl ParentElement for InfoCard {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for InfoCard {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for InfoCard {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for InfoCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            title,
            spoken,
            children,
            borderless,
            stretch,
            plain,
            untitled,
            flush,
        } = self;

        let theme = *cx.theme();
        let overrides = std::mem::take(base.style());

        let mut card = base
            .flex()
            .flex_col()
            .min_w_0()
            .overflow_hidden()
            .when(stretch, |this| this.h_full())
            .when(!flush, |this| this.gap_4())
            .when(flush, |this| this.py_1())
            .when(!plain, |this| {
                this.when(!flush, |this| this.p_5())
                    .rounded(theme.radius)
                    .bg(theme.secondary.opacity(0.45))
                    .when(!borderless, |this| {
                        this.border_1().border_color(theme.border)
                    })
            })
            .when(!untitled, |this| this.child(eyebrow(title.clone(), cx)))
            .children(children);

        card.style().refine(&overrides);
        match spoken {
            Some(id) => card
                .id(id)
                .role(Role::Group)
                .aria_label(title)
                .into_any_element(),
            None => card.into_any_element(),
        }
    }
}

#[derive(IntoElement)]
pub struct Fact {
    base: Div,
    label: SharedString,
    value: SharedString,
    /// The id under which the label and the value reach screen readers, which they do only once
    /// given one.
    spoken: Option<ElementId>,
    striped: bool,
}

impl Fact {
    pub fn new(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            label: label.into(),
            value: value.into(),
            spoken: None,
            striped: false,
        }
    }

    pub fn striped(mut self, striped: bool) -> Self {
        self.striped = striped;
        self
    }

    /// Exposes the label and the value to screen readers as two lines of text, under ids derived
    /// from `id`. Without it both are drawn but never read.
    pub fn spoken(mut self, id: impl Into<ElementId>) -> Self {
        self.spoken = Some(id.into());
        self
    }
}

impl Styled for Fact {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Fact {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl RenderOnce for Fact {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            label,
            value,
            spoken,
            striped,
        } = self;

        let theme = *cx.theme();
        let overrides = std::mem::take(base.style());

        let mut fact = base
            .flex()
            .items_start()
            .justify_between()
            .gap_4()
            .min_w_0()
            .px(theme.metrics.pad)
            .py(theme.metrics.pad / 2.)
            .rounded(theme.radius)
            .when(striped, |this| this.bg(theme.table_hover.opacity(0.35)))
            .child(said(
                div().flex_1().min_w_0().text_color(theme.muted_foreground),
                spoken.clone().map(|id| ElementId::from((id, "label"))),
                label,
            ))
            .child(said(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_right()
                    .font_weight(FontWeight::MEDIUM),
                spoken.map(|id| ElementId::from((id, "value"))),
                value,
            ));

        fact.style().refine(&overrides);
        fact
    }
}

/// Draws `text` in `line`, as a line screen readers reach under `id` when there is one.
fn said(line: Div, id: Option<ElementId>, text: SharedString) -> AnyElement {
    match id {
        Some(id) => line
            .id(id)
            .role(Role::Label)
            .aria_value(text.clone())
            .child(text)
            .into_any_element(),
        None => line.child(text).into_any_element(),
    }
}
