//! One toast card: an icon, spinner or download ring, a message with an optional muted line
//! under it, up to two buttons and a close button. The stack that piles and times toasts
//! belongs to the views.

use rune::{El, Link, LinkPaint, Span, TextStyle, Tx, col, icon, rich, row, text};

use crate::arrow_ring::ArrowRing;
use crate::button::Button;
use crate::label::Weight;
use crate::metrics::Text;
use crate::modal::corners;
use crate::paint::{Press, rems, shadow_lg};
use crate::spinner::Spinner;
use crate::theme::Theme;

const ICON: f32 = 16.;
const BORDER: f32 = 1.;

/// A toast. The card fills the width it is given, and a caller that holds it to a height clips
/// its contents, which is how a stack piles cards behind the front one.
pub struct Toast {
    key: String,
    message: String,
    strong: Option<String>,
    detail: Option<String>,
    icon: Option<String>,
    loading: bool,
    progress: Option<f32>,
    single_line: bool,
    action: Option<(String, Press)>,
    action_icon: Option<String>,
    cancel: Option<(String, Press)>,
    shown: f32,
    dismiss: Option<Press>,
    open: Option<Summon>,
    inert: bool,
}

/// The handler a press on the name runs, held until the card is built.
type Summon = Box<dyn FnOnce(El) -> El>;

impl Toast {
    pub fn new(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            message: message.into(),
            strong: None,
            detail: None,
            icon: None,
            loading: false,
            progress: None,
            single_line: false,
            action: None,
            action_icon: None,
            cancel: None,
            shown: 1.,
            dismiss: None,
            open: None,
            inert: false,
        }
    }

    /// A name inside the message to set in bold, and to underline when [`Toast::on_open`] opens
    /// what it names.
    pub fn strong(mut self, name: impl Into<String>) -> Self {
        self.strong = Some(name.into());
        self
    }

    /// A muted line below the message, wrapping rather than cut short, such as why it failed.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn icon(mut self, path: impl Into<String>) -> Self {
        self.icon = Some(path.into());
        self
    }

    /// Shows a turning spinner in place of the icon.
    pub fn loading(mut self) -> Self {
        self.loading = true;
        self
    }

    /// Shows a download ring filling `done` of the way round in place of the icon, and the
    /// percentage after the message.
    pub fn progress(mut self, done: f32) -> Self {
        self.progress = Some(done.clamp(0., 1.));
        self
    }

    /// Keeps the message to one line, cut short with an ellipsis, rather than wrapping it.
    pub fn single_line(mut self) -> Self {
        self.single_line = true;
        self
    }

    /// The outlined button beside the message, such as Undo.
    pub fn action<S: 'static>(
        mut self,
        label: impl Into<String>,
        f: impl Fn(&mut S, &mut Tx) + 'static,
    ) -> Self {
        let mut press = Press::default();
        press.press_tx(f);
        self.action = Some((label.into(), press));
        self
    }

    /// An icon after the action's label, such as an arrow on a button that opens a page.
    pub fn action_icon(mut self, path: impl Into<String>) -> Self {
        self.action_icon = Some(path.into());
        self
    }

    /// The quiet button before the action, such as Cancel.
    pub fn cancel<S: 'static>(
        mut self,
        label: impl Into<String>,
        f: impl Fn(&mut S, &mut Tx) + 'static,
    ) -> Self {
        let mut press = Press::default();
        press.press_tx(f);
        self.cancel = Some((label.into(), press));
        self
    }

    /// Keeps the buttons drawn but unpressable, for a card on its way out.
    pub fn inert(mut self) -> Self {
        self.inert = true;
        self
    }

    /// How much of the contents show, from 0 to 1, while the card itself stays. A card stacked
    /// behind another hides its contents; the change fades.
    pub fn contents(mut self, shown: f32) -> Self {
        self.shown = shown.clamp(0., 1.);
        self
    }

    pub fn on_dismiss<S: 'static>(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        let mut press = Press::default();
        press.press_tx(f);
        self.dismiss = Some(press);
        self
    }

    /// Opens what the [`Toast::strong`] name stands for, from a press on the name.
    pub fn on_open<S: 'static>(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.open = Some(Box::new(move |e: El| {
            e.on_link(move |s: &mut S, _: &str, tx: &mut Tx| f(s, tx))
        }));
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let pad = theme.metrics.pad * 2.;
        let style = TextStyle::new(theme.text(Text::Label), Weight::NORMAL);
        let percent = self.progress.map(|done| {
            i18n::t!("toast-percent", percent = (done * 100.).floor() as usize).into_owned()
        });
        let about = match (self.detail.clone(), percent.clone()) {
            (Some(detail), Some(percent)) => Some(format!("{detail}, {percent}")),
            (detail, percent) => detail.or(percent),
        };
        let leading = match (self.progress, self.loading, &self.icon) {
            (Some(done), _, _) => Some(ArrowRing::new(Some(done)).color(theme.primary).build(theme)),
            (None, true, _) => Some(
                Spinner::new()
                    .size(ICON)
                    .color(theme.popover_foreground)
                    .build(theme),
            ),
            (None, false, Some(path)) => Some(
                icon(icons::get(path), ICON)
                    .color(theme.popover_foreground.color())
                    .a11y_hidden(),
            ),
            (None, false, None) => None,
        };

        let mut message = message(&self.message, self.strong.as_deref(), self.open.is_some(), theme)
            .font(TextStyle::new(theme.text(Text::Label), Weight::MEDIUM))
            .color(theme.popover_foreground.color())
            .min_w(0.);
        message = match self.single_line {
            true => message.truncate(),
            false => message.wrap(),
        };
        if let Some(open) = self.open.filter(|_| !self.inert) {
            message = open(message);
        }
        let mut words = col()
            .grow(1.)
            .shrink()
            .min_w(0.)
            .gap(rems(theme, 0.125))
            .child(message);
        if let Some(detail) = self.detail {
            words = words.child(
                text(detail)
                    .font(style)
                    .color(theme.muted_foreground.color())
                    .wrap()
                    .a11y_hidden(),
            );
        }
        let live = !self.inert;
        let mut body = row()
            .key("body")
            .center_y()
            .gap(rems(theme, 0.75))
            .w_full()
            .padding(pad)
            .opacity(self.shown)
            .children(leading)
            .child(words);
        if let Some(percent) = percent {
            body = body.child(
                text(percent)
                    .font(style)
                    .color(theme.muted_foreground.color())
                    .a11y_hidden(),
            );
        }
        if self.cancel.is_some() || self.action.is_some() {
            let mut buttons = row().center_y().gap(rems(theme, 0.375));
            if let Some((label, press)) = self.cancel {
                let button = Button::new("cancel").ghost().small().label(label).build(theme);
                buttons = buttons.child(press.apply(button, live));
            }
            if let Some((label, press)) = self.action {
                let mut button = Button::new("action").outline().small().label(label);
                if let Some(path) = self.action_icon {
                    button = button.trailing(path);
                }
                buttons = buttons.child(press.apply(button.build(theme), live));
            }
            body = body.child(buttons);
        }
        if let Some(press) = self.dismiss {
            let button = Button::new("dismiss")
                .ghost()
                .small()
                .icon("icons/x.svg")
                .tooltip(i18n::t!("common-dismiss"))
                .build(theme);
            body = body.child(press.apply(button, live));
        }

        let mut toast = col()
            .key(self.key)
            .role("status")
            .label(self.message)
            .w_full()
            .radius(corners(theme.radius))
            .clip_rounded(corners(theme.radius))
            .border(BORDER, theme.border.color())
            .fill(theme.popover.color())
            .child(body);
        toast = shadow_lg(toast);
        match about {
            Some(about) => toast.description(about),
            None => toast,
        }
    }
}

/// The message as one run of text, with the name in it bold, and underlined and pressable
/// when it opens something.
fn message(message: &str, strong: Option<&str>, opens: bool, theme: &Theme) -> El {
    let named = strong.and_then(|name| message.find(name).map(|at| at..at + name.len()));
    let Some(named) = named else {
        return text(message.to_owned()).a11y_hidden();
    };
    let mut span = Span::new(named.start, named.end).weight(Weight::BOLD);
    if opens {
        span = span.underline(theme.popover_foreground.color());
    }
    let words = rich(message.to_owned(), vec![span]);
    match opens {
        true => words.link(Link::new(named, "open").hover(LinkPaint::default())),
        false => words.a11y_hidden(),
    }
}
