use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Div, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, LayoutId, Pixels, SharedString, Size, Stateful, Style, StyleRefinement,
    Window, div, px,
};

/// What a truncated line puts between its links.
const SEPARATOR: &str = ", ";
/// What a truncated line ends with when some of its links are left out.
const MORE: &str = ", …";
/// How far a line may measure past its box and still count as fitting, since the box is rounded
/// to device pixels while the measured text is not.
const SLACK: Pixels = px(1.);
/// How many characters of a link have to show before it is worth ellipsizing rather than
/// leaving out.
const STUB: usize = 3;

#[derive(Clone, Debug)]
pub struct InlineLink {
    pub label: SharedString,
    pub value: Option<SharedString>,
}

impl InlineLink {
    pub fn new(label: impl Into<SharedString>, value: Option<SharedString>) -> Self {
        Self {
            label: label.into(),
            value,
        }
    }
}

type ClickHandler = Rc<dyn Fn(SharedString, &mut App) + 'static>;

/// A comma-separated line of links, such as a track's artists. A truncated line shows the leading
/// links that fit whole and ends in ", …" when some are left out, and when not even the first
/// one fits with that mark, the first one ellipsizes alone.
#[derive(IntoElement)]
pub struct InlineLinks {
    base: Div,
    id: SharedString,
    items: Vec<InlineLink>,
    fallback: SharedString,
    color: Hsla,
    text_size: Option<Pixels>,
    clip: bool,
    on_click: Option<ClickHandler>,
}

impl InlineLinks {
    pub fn new(
        id: impl Into<SharedString>,
        items: impl IntoIterator<Item = InlineLink>,
        fallback: impl Into<SharedString>,
        color: Hsla,
    ) -> Self {
        Self {
            base: div(),
            id: id.into(),
            items: items.into_iter().collect(),
            fallback: fallback.into(),
            color,
            text_size: None,
            clip: false,
            on_click: None,
        }
    }

    pub fn text_size(mut self, text_size: Pixels) -> Self {
        self.text_size = Some(text_size);
        self
    }

    pub fn truncate(mut self) -> Self {
        self.clip = true;
        self
    }

    pub fn on_click(mut self, handler: impl Fn(SharedString, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl Styled for InlineLinks {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for InlineLinks {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            id,
            items,
            fallback,
            color,
            text_size,
            clip,
            on_click,
        } = self;
        let overrides = std::mem::take(base.style());

        // A clipped list ellipsizes its text rather than hiding overflow, so a text shadow
        // set on it is not cut off at the line box.
        let mut links = base
            .flex()
            .min_w_0()
            .when(!clip, |this| this.overflow_hidden())
            .text_color(color)
            .when_some(text_size, |this, text_size| this.text_size(text_size))
            .when(clip, |this| this.whitespace_nowrap());
        let count = items.len();
        links = match (count, clip) {
            (0, _) => {
                // the fallback is static text, so it is spoken as a label named by its value
                let spoken = div()
                    .id((ElementId::Name(id), "fallback"))
                    .role(gpui::accesskit::Role::Label)
                    .aria_value(fallback.clone());
                match clip {
                    true => links.child(spoken.min_w_0().text_ellipsis().child(fallback)),
                    false => links.child(spoken.child(fallback)),
                }
            }
            (1, _) => links.children(items.into_iter().map(|item| {
                link(&id, 0, 1, item, on_click.clone(), clip)
                    .flex_shrink(1.)
                    .into_any_element()
            })),
            (_, true) => links.child(Fitted::new(id, items, on_click)),
            (_, false) => links.children(items.into_iter().enumerate().map(|(index, item)| {
                joined(
                    index,
                    link(&id, index, count, item, on_click.clone(), false),
                )
            })),
        };
        links.style().refine(&overrides);
        links
    }
}

/// How many of a truncated line's links show.
#[derive(Clone, Copy)]
enum Fit {
    /// Every link fits whole.
    All,
    /// This many leading links show whole and the next one ellipsizes into the given width.
    Partial(usize, Pixels),
    /// This many leading links fit whole together with the closing ", …".
    Lead(usize),
    /// Only the first link shows, ellipsized to the given width.
    First(Pixels),
}

/// The links of a truncated line of several, laid out once its width is known. It asks for the
/// width of every link in a row and the height of one line, and picks which links show from the
/// width it is given.
struct Fitted {
    id: SharedString,
    items: Vec<InlineLink>,
    on_click: Option<ClickHandler>,
    child: Option<AnyElement>,
}

/// The measured text of a [`Fitted`] line, taken in the text style the line inherits. A stub is
/// the narrowest a link may ellipsize to, its first few characters and an ellipsis.
struct Measured {
    labels: Vec<Pixels>,
    stubs: Vec<Pixels>,
    separator: Pixels,
    more: Pixels,
}

impl Fitted {
    fn new(id: SharedString, items: Vec<InlineLink>, on_click: Option<ClickHandler>) -> Self {
        Self {
            id,
            items,
            on_click,
            child: None,
        }
    }

    /// The row for `fit`, sized to the line's own box.
    fn row(&self, fit: Fit, size: Size<Pixels>) -> AnyElement {
        let count = self.items.len();
        let shown = match fit {
            Fit::All => count,
            Fit::Partial(lead, _) => lead + 1,
            Fit::Lead(lead) => lead,
            Fit::First(_) => 1,
        };
        let hidden = self.items[shown..]
            .iter()
            .map(|item| item.label.as_ref())
            .collect::<Vec<_>>()
            .join(SEPARATOR);
        div()
            .flex()
            .w(size.width)
            .h(size.height)
            .children(self.items[..shown].iter().enumerate().map(|(index, item)| {
                let link = link(
                    &self.id,
                    index,
                    count,
                    item.clone(),
                    self.on_click.clone(),
                    true,
                );
                match fit {
                    Fit::First(width) => link.flex_shrink(1.).max_w(width).into_any_element(),
                    Fit::Partial(lead, width) if index == lead => div()
                        .flex()
                        .flex_none()
                        .child(div().flex_none().child(SEPARATOR))
                        .child(link.flex_shrink(1.).max_w(width))
                        .into_any_element(),
                    _ => joined(index, link),
                }
            }))
            .when(matches!(fit, Fit::Lead(_)), |this| {
                // the hidden links are named here, so a screen reader still hears them all
                this.child(
                    div()
                        .id((ElementId::Name(self.id.clone()), "more"))
                        .role(gpui::accesskit::Role::Label)
                        .aria_value(hidden)
                        .flex_none()
                        .child(MORE),
                )
            })
            .into_any_element()
    }
}

impl Measured {
    /// The width of the first `count` links in a row.
    fn lead(&self, count: usize) -> Pixels {
        let labels = self.labels[..count]
            .iter()
            .fold(Pixels::ZERO, |sum, width| sum + *width);
        labels + self.separator * count.saturating_sub(1) as f32
    }

    /// Shows as many links whole as fit, then as much of the next one as still reads, then a
    /// closing ", …", and only then the first link alone.
    fn fit(&self, width: Pixels) -> Fit {
        let count = self.labels.len();
        let room = width + SLACK;
        if self.lead(count) <= room {
            return Fit::All;
        }
        let whole = (1..count)
            .rev()
            .find(|&lead| self.lead(lead) <= room)
            .unwrap_or(0);
        for lead in (1..=whole).rev() {
            let rest = width - self.lead(lead) - self.separator;
            if rest + SLACK >= self.stubs[lead] {
                return Fit::Partial(lead, rest);
            }
            if self.lead(lead) + self.more <= room {
                return Fit::Lead(lead);
            }
        }
        // a first link that would fit whole is cut short, so the line still ends in "…"
        Fit::First(width.min(self.labels[0] - SLACK))
    }
}

impl Element for Fitted {
    type RequestLayoutState = Measured;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Measured) {
        let text = window.text_style();
        let rem = window.rem_size();
        let font_size = text.font_size.to_pixels(rem);
        let measure = |value: SharedString| {
            let run = text.to_run(value.len());
            window
                .text_system()
                .shape_line(value, font_size, &[run], None)
                .width()
        };
        let measured = Measured {
            labels: self
                .items
                .iter()
                .map(|item| measure(item.label.clone()))
                .collect(),
            stubs: self
                .items
                .iter()
                .map(|item| {
                    let stub = item.label.chars().take(STUB).collect::<String>();
                    measure(format!("{stub}…").into())
                })
                .collect(),
            separator: measure(SharedString::new_static(SEPARATOR)),
            more: measure(SharedString::new_static(MORE)),
        };
        let mut style = Style::default();
        style.size.width = measured.lead(self.items.len()).into();
        style.size.height = text.line_height_in_pixels(rem).into();
        style.min_size.width = Pixels::ZERO.into();
        style.flex_shrink = 1.;
        (window.request_layout(style, None, cx), measured)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        measured: &mut Measured,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut child = self.row(measured.fit(bounds.size.width), bounds.size);
        child.layout_as_root(Size::<AvailableSpace>::from(bounds.size), window, cx);
        child.prepaint_at(bounds.origin, window, cx);
        self.child = Some(child);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _measured: &mut Measured,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = self.child.as_mut() {
            child.paint(window, cx);
        }
    }
}

impl IntoElement for Fitted {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// One link of the line, a link when it has a value and a click handler, a label otherwise.
fn link(
    id: &SharedString,
    index: usize,
    count: usize,
    item: InlineLink,
    on_click: Option<ClickHandler>,
    clip: bool,
) -> Stateful<Div> {
    let InlineLink { label, value } = item;
    let linking = on_click.is_some();
    let item = div()
        .id(ElementId::NamedInteger(id.clone(), index as u64))
        .aria_position_in_set(index + 1)
        .aria_size_of_set(count)
        .min_w_0()
        .when(clip, |this| this.text_ellipsis());
    match value {
        Some(value) => item
            .role(match linking {
                true => gpui::accesskit::Role::Link,
                false => gpui::accesskit::Role::Label,
            })
            .map(|this| match linking {
                true => this.aria_label(label.clone()),
                false => this.aria_value(label.clone()),
            })
            .hover(|style| style.underline())
            .cursor_pointer()
            .when_some(on_click, |this, handler| {
                this.on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    handler(value.clone(), cx);
                })
            })
            .child(label),
        None => item
            .role(gpui::accesskit::Role::Label)
            .aria_value(label.clone())
            .child(label),
    }
}

/// A link of a line of several, behind the separator unless it leads.
fn joined(index: usize, link: Stateful<Div>) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .min_w_0()
        .when(index > 0, |this| {
            this.child(div().flex_none().child(SEPARATOR))
        })
        .child(link)
        .into_any_element()
}
