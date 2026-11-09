//! A row of one-character slots for a short numeric code, the way shadcn's InputOTP draws
//! them, and the field that types into them.

use std::rc::Rc;

use rune::{Color, Cursor, El, TextEdit, TextStyle, Tx, el, row, text, text_input};

use crate::label::Weight;
use crate::metrics::Text;
use crate::paint::rems;
use crate::theme::Theme;
use crate::time::TABULAR;

/// The caret in an empty slot and the ring round the slot being typed in.
const CARET: f32 = 1.5;
const RING: f32 = 2.;
/// The dash between two groups of slots.
const DASH: f32 = 8.;
const DASH_HEIGHT: f32 = 2.;
/// How much larger a slot is than a text field is tall, so a code reads at a glance.
const SLOT: f32 = 1.3;
const BORDER: f32 = 1.;

type Access<S> = Rc<dyn for<'a> Fn(&'a mut S) -> &'a mut TextEdit>;

/// The slots for `value`, one per character, in groups that share their borders with a dash
/// between groups. A screen that only shows a code draws it with this; [`OtpInput`] types into
/// it.
pub struct OtpSlots {
    value: String,
    length: usize,
    group: usize,
    /// The slot the next digit lands in, ringed while the field has focus.
    active: Option<usize>,
}

impl OtpSlots {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
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

    pub fn build(self, theme: &Theme) -> El {
        let side = theme.metrics.field * SLOT;
        let size = theme.text(Text::Large);
        let group = match self.group {
            0 => self.length.max(1),
            size => size,
        };
        let characters: Vec<char> = self.value.chars().collect();
        let style = TextStyle::new(size, Weight::NORMAL).features(TABULAR);
        let slot = |index: usize| {
            let first = index.is_multiple_of(group);
            let last = index % group == group - 1 || index + 1 == self.length;
            let radius = |on: bool| match on {
                true => theme.radius,
                false => 0.,
            };
            let corners = [radius(first), radius(last), radius(last), radius(first)];
            let character = characters.get(index).copied();
            let active = self.active == Some(index);
            let left = match first {
                true => BORDER,
                false => 0.,
            };
            let mut slot = row()
                .key(format!("slot-{index}"))
                .center()
                .size(side, side)
                .fill(theme.secondary.color())
                .border_sides([BORDER, BORDER, BORDER, left], theme.border.color())
                .corners(corners)
                .a11y_hidden();
            if let Some(character) = character {
                slot = slot.child(
                    text(character.to_string())
                        .font(style)
                        .color(theme.foreground.color()),
                );
            }
            if active {
                // the ring covers the border the slot before drew on this one's left
                let reach = match first {
                    true => 0.,
                    false => -BORDER,
                };
                slot = slot.child(
                    el().pin(Some(reach), Some(0.), Some(0.), Some(0.))
                        .border(RING, theme.muted_foreground.color())
                        .corners(corners),
                );
                if character.is_none() {
                    slot = slot.child(el().w(CARET).h(size).fill(theme.foreground.color()));
                }
            }
            slot
        };
        let mut slots = row().center_y().gap(rems(theme, 0.5)).a11y_hidden();
        let mut start = 0;
        while start < self.length {
            let end = (start + group).min(self.length);
            if start > 0 {
                slots = slots.child(
                    el().w(DASH)
                        .h(DASH_HEIGHT)
                        .radius(DASH_HEIGHT / 2.)
                        .fill(theme.muted_foreground.color()),
                );
            }
            slots = slots.child(row().children((start..end).map(slot)));
            start = end;
        }
        slots
    }
}

/// A field for a short numeric code that shows each digit in a slot of its own. It keeps only
/// digits, never more than `length` of them, and pasted text gives up its digits too. The owner
/// reads the code from its `TextEdit` and acts once it is `length` long.
pub struct OtpInput<S> {
    key: String,
    edit: TextEdit,
    access: Access<S>,
    length: usize,
    group: usize,
    name: String,
}

impl<S: 'static> OtpInput<S> {
    /// A field of `length` slots over `edit`, which `access` finds in the window's state, named
    /// `name` for screen readers.
    pub fn new(
        key: impl Into<String>,
        edit: &TextEdit,
        length: usize,
        name: impl Into<String>,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut TextEdit + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            edit: edit.clone(),
            access: Rc::new(access),
            length,
            group: 0,
            name: name.into(),
        }
    }

    /// Splits the slots into groups of `size`, with a dash between them.
    pub fn grouped(mut self, size: usize) -> Self {
        self.group = size;
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let value: String = self.edit.text.chars().take(self.length).collect();
        let active = self
            .edit
            .focused
            .then(|| value.chars().count().min(self.length.saturating_sub(1)));
        let length = self.length;
        let edits = self.access.clone();
        let filter = self.access.clone();
        // The editor takes the keys, the caret and the clipboard; it draws nothing itself, and
        // the slots under it show what it holds.
        let field = text_input(
            &self.edit,
            TextStyle::new(theme.text(Text::Large), Weight::NORMAL),
        )
        .single_line()
        .key("code")
        .label(self.name)
        .pin(Some(0.), Some(0.), Some(0.), Some(0.))
        .color(Color::TRANSPARENT)
        .caret(Color::TRANSPARENT, Color::TRANSPARENT)
        .on_edit(move |s: &mut S| edits(s))
        .on_input_change(move |s: &mut S, _: &mut Tx| {
            let edit = filter(s);
            let digits: String = edit
                .text
                .chars()
                .filter(char::is_ascii_digit)
                .take(length)
                .collect();
            if digits != edit.text {
                edit.replace_text(digits);
            }
        });
        row()
            .key(self.key)
            .cursor(Cursor::Text)
            .child(
                OtpSlots::new(value)
                    .length(self.length)
                    .grouped(self.group)
                    .active(active)
                    .build(theme),
            )
            .child(field)
    }
}
