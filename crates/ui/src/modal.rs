//! A dialog over a dimmed window: a title, an optional detail, a body that scrolls and a row of
//! actions. It holds the keyboard while open and hands focus back to its opener when it closes.

use crate::smooth::Smooth as _;
use std::rc::Rc;

use rune::{El, ScrollSpec, TextStyle, Tx, col, el, row, text};

use crate::button::Button;
use crate::label::Weight;
use crate::metrics::{Rounding, Text};
use crate::motion::Springs;
use crate::overlay::bloom;
use crate::paint::shadow_md;
use crate::theme::Theme;

/// How much of the background colour dims the window behind a dialog.
const BACKDROP: f32 = 0.8;
/// How wide a dialog is, in covers.
const WIDTH: f32 = 2.4;
/// How many times the theme radius a dialog's corners take, capped at what Rounded gives.
const ROUNDING: f32 = 2.;
const BORDER: f32 = 1.;
/// How far a scrolling body fades its edges while content passes under them.
const FADE: f32 = 16.;

type Dismiss<S> = Rc<dyn Fn(&mut S, &mut Tx)>;

/// The corner radius of a large floating surface for a theme radius: `ROUNDING` times it,
/// capped at what the Rounded setting gives. Dialogs, toasts and notices share it.
pub fn corners(radius: f32) -> f32 {
    (radius * ROUNDING).min(Rounding::Rounded.radius() * ROUNDING)
}

/// A dialog. Mount it while it should show with [`Modal::build`]; it pushes a dimming backdrop
/// and the panel into the layer. Escape and a press on the backdrop dismiss it.
pub struct Modal<S> {
    key: String,
    title: String,
    detail: Option<String>,
    body: Vec<El>,
    actions: Vec<El>,
    dismiss: Option<Dismiss<S>>,
    close_button: bool,
    width: Option<f32>,
}

impl<S: 'static> Modal<S> {
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            detail: None,
            body: Vec::new(),
            actions: Vec::new(),
            dismiss: None,
            close_button: false,
            width: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn child(mut self, body: El) -> Self {
        self.body.push(body);
        self
    }

    /// A button in the row along the foot of the dialog, the last one at the end.
    pub fn action(mut self, action: El) -> Self {
        self.actions.push(action);
        self
    }

    /// Draws a cross in the top corner, which dismisses the dialog as a press outside does.
    pub fn close_button(mut self) -> Self {
        self.close_button = true;
        self
    }

    /// The panel's width in place of the default, two and a bit covers.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn on_dismiss(mut self, f: impl Fn(&mut S) + 'static) -> Self {
        self.dismiss = Some(Rc::new(move |s: &mut S, _: &mut Tx| f(s)));
        self
    }

    pub fn on_dismiss_tx(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.dismiss = Some(Rc::new(f));
        self
    }

    pub fn build(self, theme: &Theme, layer: &mut Vec<El>) {
        let pad = theme.metrics.pad;
        let room = pad * 2.;
        // with no action row under it the body carries the bottom padding itself, so the
        // panel is inset by the same amount all the way round
        let tail = match self.actions.is_empty() {
            true => room,
            false => pad,
        };
        let mut words = col()
            .grow(1.)
            .shrink()
            .min_w(0.)
            .gap(crate::paint::rems(theme, 0.25))
            .child(
                text(self.title.clone())
                    .font(TextStyle::new(theme.text(Text::Large), Weight::SEMIBOLD))
                    .color(theme.popover_foreground.color())
                    .wrap()
                    .a11y_hidden(),
            );
        if let Some(detail) = &self.detail {
            words = words.child(
                text(detail.clone())
                    .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .wrap()
                    .a11y_hidden(),
            );
        }
        let mut head = row()
            .key("head")
            .items_start()
            .gap(pad)
            .px(room)
            .pt(room)
            .pb(pad)
            .child(words);
        if let Some(dismiss) = self.dismiss.clone().filter(|_| self.close_button) {
            head = head.child(
                Button::new("close")
                    .icon("icons/x.svg")
                    .small()
                    .tooltip(i18n::t!("common-dismiss"))
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| dismiss(s, tx))
                    .build(theme),
            );
        }
        let mut panel = col()
            .key(format!("{}-dialog", self.key))
            .role("dialog")
            .label(self.title)
            .w(self.width.unwrap_or(theme.metrics.cover * WIDTH))
            .shrink()
            .min_h(0.)
            .radius(corners(theme.radius))
            .clip_rounded(corners(theme.radius))
            .border(BORDER, theme.border.color())
            .fill(theme.popover.color())
            .motion(Springs::BASE)
            .transition(bloom())
            .child(head);
        panel = shadow_md(panel);
        if let Some(detail) = self.detail {
            panel = panel.description(detail);
        }
        if !self.body.is_empty() {
            panel = panel.child(
                col()
                    .key("body")
                    .w_full()
                    .shrink()
                    .min_h(0.)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .smooth::<S>("body")
                    .scroll_edge_fade(FADE)
                    .child(
                        col()
                            .w_full()
                            .min_w(0.)
                            .gap(pad)
                            .px(room)
                            .pt(pad)
                            .pb(tail)
                            .children(self.body),
                    ),
            );
        }
        if !self.actions.is_empty() {
            panel = panel.child(
                row()
                    .key("actions")
                    .justify_end()
                    .gap(pad)
                    .padding(room)
                    .pt(pad)
                    .children(self.actions),
            );
        }
        let backdrop = el()
            .key(format!("{}-backdrop", self.key))
            .absolute(0., 0.)
            .w_full()
            .h_full()
            .fill(theme.background.opacity(BACKDROP).color())
            .transition(rune::Transition::opacity())
            .a11y_hidden();
        let panel = match self.dismiss {
            Some(dismiss) => panel.modal_tx(true, move |s: &mut S, tx: &mut Tx| dismiss(s, tx)),
            None => panel.modal(false, |_: &mut S| {}),
        };
        layer.push(backdrop);
        layer.push(
            col()
                .key(format!("{}-frame", self.key))
                .absolute(0., 0.)
                .w_full()
                .h_full()
                .padding(theme.metrics.inset)
                .center()
                .child(panel),
        );
    }
}
