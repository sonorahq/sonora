//! The single-line text field: Rune's own `text_input` editor in Sonora's frame, with a leading
//! icon, a placeholder, a unit after the text, a clear button and the shortcut that focuses it.

use std::rc::Rc;

use rune::{Align, Cursor, El, KeyEvent, TextEdit, TextStyle, Tx, row, text, text_input};

use crate::button::Button;
use crate::glass::{blurring, glass};
use crate::kbd::Kbd;
use crate::label::Weight;
use crate::metrics::{Text, tucked};
use crate::motion::Springs;
use crate::paint::{rems, shadow_sm};
use crate::theme::Theme;

const ICON: f32 = 16.;
const BORDER: f32 = 1.;
/// The ring drawn round the field while it has focus.
const RING: f32 = 2.;
const RING_OFFSET: f32 = 1.;
const RING_ALPHA: f32 = 0.5;
/// How strongly a selection shows its colour under the text.
const SELECTION: f32 = 0.35;
/// The clear button's side padding, tighter than a small button's own.
const CLEAR_INSET: f32 = 4.;

/// Where the field finds the `TextEdit` it shows in the window's state.
type Access<S> = Rc<dyn for<'a> Fn(&'a mut S) -> &'a mut TextEdit>;
type Submit<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type Keys<S> = Rc<dyn Fn(&mut S, &KeyEvent, &mut Tx) -> bool>;

/// A text field over a `TextEdit` the window's state owns. The runtime moves the caret, selects,
/// copies, pastes, undoes and composes; the field binds those edits back through
/// [`Input::new`]'s accessor. To focus it, set `TextEdit::focused` once in state, or chain
/// `.autofocus()` on a field that should take focus as it appears.
pub struct Input<S> {
    key: String,
    edit: TextEdit,
    access: Access<S>,
    hint: String,
    /// What a screen reader calls the field, when the hint is an example rather than that.
    name: Option<String>,
    icon: Option<String>,
    compact: bool,
    tucked: bool,
    blurred: bool,
    clearable: bool,
    masked: bool,
    end: bool,
    invalid: bool,
    suffix: Option<String>,
    shortcut: Option<String>,
    autofocus: bool,
    submit: Option<Submit<S>>,
    keys: Option<Keys<S>>,
    change: Option<Submit<S>>,
}

impl<S: 'static> Input<S> {
    /// A field showing `edit` with the placeholder `hint`, already resolved. `access` finds the
    /// same `TextEdit` in the window's state, where the field writes its edits back.
    pub fn new(
        key: impl Into<String>,
        edit: &TextEdit,
        hint: impl Into<String>,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut TextEdit + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            edit: edit.clone(),
            access: Rc::new(access),
            hint: hint.into(),
            name: None,
            icon: None,
            compact: false,
            tucked: false,
            blurred: false,
            clearable: false,
            masked: false,
            end: false,
            invalid: false,
            suffix: None,
            shortcut: None,
            autofocus: false,
            submit: None,
            keys: None,
            change: None,
        }
    }

    /// An icon before the text, such as a magnifier on a search field.
    pub fn icon(mut self, path: impl Into<String>) -> Self {
        self.icon = Some(path.into());
        self
    }

    /// As tall as a small control rather than a form field.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Sits inside a padded surface such as a menu: concentric corners, no border, full width.
    pub fn tucked(mut self) -> Self {
        self.tucked = true;
        self
    }

    /// A field that floats over content: it casts a soft shadow and draws as frosted glass while
    /// the blur setting is on.
    pub fn blurred(mut self) -> Self {
        self.blurred = true;
        self
    }

    /// A button at the end that empties the field while it holds text.
    pub fn clearable(mut self) -> Self {
        self.clearable = true;
        self
    }

    /// Draws the text as dots, for a password or a key. Secure fields keep no undo and offer
    /// nothing to copy.
    pub fn masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }

    /// Names the field for screen readers when the hint is an example rather than what the
    /// field asks for. The hint is then read as the placeholder.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Keeps the text against the field's end, so numbers in fields stacked together line up by
    /// their last digit.
    pub fn align_end(mut self) -> Self {
        self.end = true;
        self
    }

    /// Marks the field as holding something that was turned down: its border takes the danger
    /// colour and screen readers hear it as invalid.
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    /// A muted unit after the text inside the field, already resolved.
    pub fn suffix(mut self, unit: impl Into<String>) -> Self {
        self.suffix = Some(unit.into());
        self
    }

    /// The shortcut that focuses the field, spelled as key bindings are (`"secondary-f"`), drawn
    /// as keys at its end while it is empty.
    pub fn shortcut(mut self, keystroke: impl Into<String>) -> Self {
        self.shortcut = Some(keystroke.into());
        self
    }

    /// Takes focus the frame the field first appears.
    pub fn autofocus(mut self) -> Self {
        self.autofocus = true;
        self
    }

    /// Runs on Return.
    pub fn on_submit(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.submit = Some(Rc::new(f));
        self
    }

    /// Sees each key before the editor does; returning true takes it.
    pub fn on_key(mut self, f: impl Fn(&mut S, &KeyEvent, &mut Tx) -> bool + 'static) -> Self {
        self.keys = Some(Rc::new(f));
        self
    }

    /// Runs after each edit that changed the text.
    pub fn on_change(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.change = Some(Rc::new(f));
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let height = match self.compact {
            true => theme.metrics.control_small,
            false => theme.metrics.field,
        };
        let radius = match self.tucked {
            true => tucked(theme.radius, theme.font_size),
            false => theme.radius,
        };
        let ink = theme.foreground.color();
        let style = TextStyle::new(theme.text(Text::Body), Weight::NORMAL);
        let empty = self.edit.text.is_empty();
        let access = self.access.clone();
        let mut field = text_input(&self.edit, style)
            .single_line()
            .key("input")
            .grow(1.)
            .shrink()
            .min_w(0.)
            .color(ink)
            .placeholder(self.hint.clone(), theme.muted_foreground.color())
            .caret(ink, theme.selection.opacity(SELECTION).color())
            .invalid(self.invalid)
            .on_edit(move |s: &mut S| access(s));
        field = match &self.name {
            Some(name) => field.label(name.clone()),
            None if !self.hint.is_empty() => field.label(self.hint.clone()),
            None => field,
        };
        if self.masked {
            field = field.secure();
        }
        if self.end {
            field = field.align(Align::End);
        }
        if self.autofocus {
            field = field.autofocus();
        }
        if let Some(submit) = self.submit {
            field = field.on_submit(move |s: &mut S, tx: &mut Tx| submit(s, tx));
        }
        if let Some(keys) = self.keys {
            field =
                field.on_input_key(move |s: &mut S, key: &KeyEvent, tx: &mut Tx| keys(s, key, tx));
        }
        if let Some(change) = self.change {
            field = field.on_input_change(move |s: &mut S, tx: &mut Tx| change(s, tx));
        }

        let mut frame = row()
            .key(self.key)
            .role("textbox")
            .center_y()
            .gap(rems(theme, 0.5))
            .min_w(0.)
            .h(height)
            .px(rems(theme, 0.75))
            .radius(radius)
            .clip_rounded(radius)
            .cursor(Cursor::Text)
            .motion(Springs::CONTROL)
            .focus_within_ring(
                RING,
                RING_OFFSET,
                theme.muted_foreground.opacity(RING_ALPHA).color(),
            );
        frame = match (self.blurred, self.tucked, blurring(theme)) {
            (true, _, true) => glass(frame, theme),
            (false, true, true) => frame.fill(theme.secondary_hover.color()),
            _ => frame.fill(theme.secondary.color()),
        };
        if self.blurred {
            frame = shadow_sm(frame);
        }
        frame = match self.tucked {
            true => frame.w_full(),
            false => frame.w_full().shrink().border(
                BORDER,
                match self.invalid {
                    true => theme.danger.color(),
                    false => theme.border.color(),
                },
            ),
        };
        if let Some(path) = &self.icon {
            frame = frame.child(
                rune::icon(icons::get(path), ICON)
                    .color(theme.muted_foreground.color())
                    .a11y_hidden(),
            );
        }
        frame = frame.child(field);
        if let Some(unit) = self.suffix {
            frame = frame.child(
                text(unit)
                    .font(TextStyle::new(theme.text(Text::Label), Weight::NORMAL))
                    .color(theme.muted_foreground.color()),
            );
        }
        if self.clearable && !empty {
            let access = self.access.clone();
            frame = frame.child(
                Button::new("clear")
                    .icon("icons/x.svg")
                    .tooltip(i18n::t!("common-clear"))
                    .small()
                    .ghost()
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                        access(s).clear();
                        tx.instant();
                    })
                    .build(theme)
                    .px(CLEAR_INSET),
            );
        }
        if let Some(keystroke) = self.shortcut.filter(|_| empty) {
            frame = frame.child(Kbd::new(keystroke).build(theme));
        }
        frame
    }
}
