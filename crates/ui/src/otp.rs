use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, ElementInputHandler, EntityInputHandler, FocusHandle, Focusable,
    MouseButton, Pixels, SharedString, StyleRefinement, UTF16Selection, Window, canvas, div, px,
};

use crate::input::{Backspace, BackspaceToStart, BackspaceWord, INPUT_CONTEXT, Paste};
use crate::theme::ActiveTheme as _;
use crate::{Text, tabular};

/// The width of the caret in an empty slot and of the ring round the slot being typed in.
const CARET: Pixels = px(1.5);
const RING: Pixels = px(2.);
/// The dash between two groups of slots.
const DASH: Pixels = px(8.);
const DASH_HEIGHT: Pixels = px(2.);
/// How much larger a slot is than a text field is tall, so a code reads at a glance.
const SLOT: f32 = 1.3;

/// A row of one-character slots for a short numeric code, the way shadcn's InputOTP draws them.
/// The slots stand in groups that share their borders, with a dash between groups. Nothing is
/// typed into them: `OtpInput` draws the code it holds with this, and a screen that only shows a
/// code draws it straight away.
#[derive(IntoElement)]
pub struct OtpSlots {
    style: StyleRefinement,
    value: SharedString,
    length: usize,
    group: usize,
    /// The slot the next digit lands in, ringed while the field has focus.
    active: Option<usize>,
}

impl OtpSlots {
    /// Slots for `value`, one per character, in a single group.
    pub fn new(value: impl Into<SharedString>) -> Self {
        let value = value.into();
        Self {
            style: StyleRefinement::default(),
            length: value.chars().count(),
            value,
            group: 0,
            active: None,
        }
    }

    /// Draws `length` slots however long the value is. The ones past its end stay empty.
    pub fn length(mut self, length: usize) -> Self {
        self.length = length;
        self
    }

    /// Splits the slots into groups of `size`, with a dash between them.
    pub fn grouped(mut self, size: usize) -> Self {
        self.group = size;
        self
    }

    pub fn active(mut self, active: Option<usize>) -> Self {
        self.active = active;
        self
    }
}

impl Styled for OtpSlots {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for OtpSlots {
    fn render(mut self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let side = theme.metrics.field * SLOT;
        let text = theme.text(Text::Large);
        let group = match self.group {
            0 => self.length.max(1),
            size => size,
        };
        let characters: Vec<char> = self.value.chars().collect();
        let overrides = std::mem::take(&mut self.style);

        let slot = |index: usize| {
            let first = index.is_multiple_of(group);
            let last = index % group == group - 1 || index + 1 == self.length;
            let character = characters.get(index).copied();
            let active = self.active == Some(index);
            div()
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(side)
                .bg(theme.secondary)
                .border_y_1()
                .border_r_1()
                .border_color(theme.border)
                .when(first, |this| this.border_l_1().rounded_l(theme.radius))
                .when(last, |this| this.rounded_r(theme.radius))
                .text_size(text)
                .font_features(tabular())
                .text_color(theme.foreground)
                .when_some(character, |this, character| {
                    this.child(SharedString::from(character.to_string()))
                })
                .when(active, |this| {
                    // the ring covers the border the slot before drew on this one's left
                    this.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right_0()
                            .map(|ring| match first {
                                true => ring.left_0(),
                                false => ring.left(-px(1.)),
                            })
                            .border(RING)
                            .border_color(theme.muted_foreground)
                            .when(first, |ring| ring.rounded_l(theme.radius))
                            .when(last, |ring| ring.rounded_r(theme.radius)),
                    )
                    .when(character.is_none(), |this| {
                        this.child(div().w(CARET).h(text).bg(theme.foreground))
                    })
                })
        };

        let mut row = div().flex().flex_none().items_center().gap_2();
        let mut start = 0;
        while start < self.length {
            let end = (start + group).min(self.length);
            if start > 0 {
                row = row.child(
                    div()
                        .flex_none()
                        .w(DASH)
                        .h(DASH_HEIGHT)
                        .rounded_full()
                        .bg(theme.muted_foreground),
                );
            }
            row = row.child(div().flex().flex_none().children((start..end).map(slot)));
            start = end;
        }
        row.style().refine(&overrides);
        row
    }
}

/// A field for a short numeric code that shows each digit in a slot of its own, like shadcn's
/// InputOTP. It keeps only digits, never more than `length` of them, and pasted text gives up
/// its digits too. The parent reads `value` and checks `complete` to act on a whole code.
pub struct OtpInput {
    focus_handle: FocusHandle,
    length: usize,
    group: usize,
    /// The Fluent key of what a screen reader calls the field.
    name: SharedString,
    value: String,
}

impl OtpInput {
    pub fn new(length: usize, name: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            length,
            group: 0,
            name: name.into(),
            value: String::new(),
        }
    }

    /// Splits the slots into groups of `size`, with a dash between them.
    pub fn grouped(mut self, size: usize) -> Self {
        self.group = size;
        self
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// Whether every slot holds a digit.
    pub fn complete(&self) -> bool {
        self.value.len() == self.length
    }

    /// Replaces the code with the digits of `text`, as many as fit.
    pub fn set_value(&mut self, text: &str, cx: &mut Context<Self>) {
        self.value.clear();
        self.push(text, cx);
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus_handle, cx);
    }

    /// Adds the digits of `text` after the ones already typed, as many as fit.
    fn push(&mut self, text: &str, cx: &mut Context<Self>) {
        let room = self.length - self.value.len();
        self.value
            .extend(text.chars().filter(char::is_ascii_digit).take(room));
        cx.notify();
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.value.pop().is_none() {
            window.play_system_bell();
        }
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.value.clear();
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.push(&text, cx);
        }
    }

    /// The whole code as a range of UTF-16 offsets, which is all an input method is shown.
    fn all(&self) -> Range<usize> {
        0..self.value.len()
    }
}

impl Focusable for OtpInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for OtpInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let active = focused.then(|| self.value.len().min(self.length.saturating_sub(1)));
        let entity = cx.entity();
        let focus = self.focus_handle.clone();

        div()
            // the focus handle doubles as the element id, as it does for `Input`
            .id(&self.focus_handle)
            .role(gpui::accesskit::Role::TextInput)
            .aria_label(i18n::lookup(&self.name, None))
            .when(!self.value.is_empty(), |this| {
                this.aria_value(SharedString::from(self.value.clone()))
            })
            .relative()
            .flex()
            .flex_none()
            .key_context(INPUT_CONTEXT)
            .track_focus(&self.focus_handle)
            .cursor_text()
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(|this, _: &BackspaceWord, _, cx| this.clear(cx)))
            .on_action(cx.listener(|this, _: &BackspaceToStart, _, cx| this.clear(cx)))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.focus(window, cx);
                }),
            )
            .child(
                OtpSlots::new(self.value.clone())
                    .length(self.length)
                    .grouped(self.group)
                    .active(active),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds: Bounds<Pixels>, _, window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

impl EntityInputHandler for OtpInput {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        actual.replace(self.all());
        Some(self.value.clone())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.value.len();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push(text, cx);
    }

    /// Text an input method is still composing is left out until it commits, which arrives
    /// through `replace_text_in_range`, so no digit lands twice.
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(bounds)
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.value.len())
    }
}
