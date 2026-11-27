use std::borrow::Cow;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Div, Entity, Pixels, Render, SharedString, Task, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use i18n::t;
use router::Destination;
use state::{Outcome, Toast, Toasts};
use ui::{
    ActiveTheme as _, Button, Look, Rounding, RowSize, Text, Theme, ThemeKind, eyebrow, heading,
};
use views::ToastStack;

const INTER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/Inter-Regular.ttf"
));

const FIRST_SIZE: gpui::Size<Pixels> = size(px(900.), px(720.));
/// The strip at the bottom the stack hangs from, standing in for the player bar.
const BAR: Pixels = px(88.);
/// How long a sample promise works before it settles.
const WORK: Duration = Duration::from_secs(2);
/// How many toasts the burst sample shows.
const BURST: usize = 5;
/// The name the samples put in bold.
const NAME: &str = "Midnight City";

/// One button on the page: its id, its label, and the toast it shows.
type Sample = (
    &'static str,
    &'static str,
    fn(&mut Page, &mut Context<Page>),
);

/// A page with a button for every kind of toast, so each one can be tried without waiting for
/// the app to need it.
struct Page {
    stack: Entity<ToastStack>,
    work: Vec<Task<()>>,
}

impl Page {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            stack: cx.new(ToastStack::new),
            work: Vec::new(),
        }
    }

    /// Shows a loading toast and settles it into a success or a failure once the work is done.
    fn promise(&mut self, succeeds: bool, cx: &mut Context<Self>) {
        let id = Toasts::post(
            Toast::new(Outcome::Loading, "toast-sample-saving").name(NAME),
            cx,
        );
        self.work.push(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(WORK).await;
            cx.update(|cx| {
                let settled = match succeeds {
                    true => Toast::new(Outcome::Done, "toast-sample-saved").name(NAME),
                    false => Toast::new(Outcome::Failed, "toast-sample-save-failed")
                        .name(NAME)
                        .detail(t!("toast-sample-save-failed-detail")),
                };
                Toasts::settle(id, settled, cx);
            });
        }));
    }

    fn group(&self, title: &'static str, samples: &[Sample], cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(eyebrow(title, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(samples.iter().map(|(id, label, show)| {
                        let show = *show;
                        Button::new(*id)
                            .outline()
                            .label(*label)
                            .on_click(cx.listener(move |this, _, _, cx| show(this, cx)))
                    })),
            )
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();

        let kinds: [Sample; 6] = [
            ("toast-plain", "Default", |_, cx| {
                Toasts::show(Outcome::Plain, "toast-sample-plain", cx)
            }),
            ("toast-done", "Success", |_, cx| {
                Toasts::show(Outcome::Done, "toast-sample-done", cx)
            }),
            ("toast-info", "Info", |_, cx| {
                Toasts::show(Outcome::Info, "toast-sample-info", cx)
            }),
            ("toast-warning", "Warning", |_, cx| {
                Toasts::show(Outcome::Warning, "toast-sample-warning", cx)
            }),
            ("toast-failed", "Error", |_, cx| {
                Toasts::show(Outcome::Failed, "toast-sample-failed", cx)
            }),
            ("toast-loading", "Loading", |_, cx| {
                Toasts::post(Toast::new(Outcome::Loading, "toast-sample-loading"), cx);
            }),
        ];
        let content: [Sample; 5] = [
            ("toast-detail", "With description", |_, cx| {
                Toasts::with_detail(
                    Outcome::Plain,
                    "toast-sample-event",
                    None,
                    t!("toast-sample-event-detail"),
                    cx,
                )
            }),
            ("toast-action", "With action", |_, cx| {
                let toast = Toast::new(Outcome::Plain, "toast-sample-event")
                    .detail(t!("toast-sample-event-detail"))
                    .action("toast-sample-undo", undo);
                Toasts::post(toast, cx);
            }),
            ("toast-cancel", "Action and cancel", |_, cx| {
                let toast = Toast::new(Outcome::Plain, "toast-sample-delete")
                    .name(NAME)
                    .cancel("toast-sample-cancel", |_| {})
                    .action("toast-sample-undo", undo);
                Toasts::post(toast, cx);
            }),
            ("toast-name", "With a name", |_, cx| {
                Toasts::about(Outcome::Done, "toast-sample-queued", NAME, cx)
            }),
            ("toast-long", "Long message", |_, cx| {
                Toasts::show(Outcome::Info, "toast-sample-long", cx)
            }),
        ];
        let behaviour: [Sample; 5] = [
            ("toast-promise", "Promise that succeeds", |this, cx| {
                this.promise(true, cx)
            }),
            ("toast-promise-failed", "Promise that fails", |this, cx| {
                this.promise(false, cx)
            }),
            ("toast-lasting", "Stays until closed", |_, cx| {
                Toasts::post(
                    Toast::new(Outcome::Info, "toast-sample-lasting").lasting(),
                    cx,
                );
            }),
            ("toast-burst", "Five at once", |_, cx| {
                for number in 1..=BURST {
                    Toasts::about(
                        Outcome::Plain,
                        "toast-sample-burst",
                        SharedString::from(number.to_string()),
                        cx,
                    );
                }
            }),
            ("toast-clear", "Dismiss all", |_, cx| Toasts::clear(cx)),
        ];

        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family("Inter")
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_size(theme.text(Text::Body))
            .child(
                div()
                    .id("toasts-page")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_6()
                            .w_full()
                            .p_6()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(heading("Toasts", cx))
                                    .child(
                                        div()
                                            .text_size(theme.text(Text::Label))
                                            .text_color(theme.muted_foreground)
                                            .child(
                                                "Every kind of toast Sonora can show. Point at \
                                                 the pile to spread it out and stop the clocks, \
                                                 or drag a toast to the right to dismiss it.",
                                            ),
                                    ),
                            )
                            .child(self.group("Kinds", &kinds, cx))
                            .child(self.group("Content", &content, cx))
                            .child(self.group("Behaviour", &behaviour, cx)),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(BAR)
                    .flex_none()
                    .border_t_1()
                    .border_color(theme.table_row_border)
                    .child(self.stack.clone()),
            )
    }
}

/// The sample action, which only says it ran.
fn undo(cx: &mut App) {
    Toasts::show(Outcome::Info, "toast-sample-undone", cx);
}

fn main() {
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(|cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![Cow::Borrowed(INTER)])
                .expect("cannot load Inter");

            Theme::init(
                Look {
                    kind: ThemeKind::Dark,
                    rounding: Rounding::Rounded,
                    font: 14.,
                    rows: RowSize::default(),
                    transparent: false,
                    transparency: 0.,
                    blur: false,
                    blur_window: false,
                    tint: None,
                    tint_secondary: None,
                },
                &Default::default(),
                cx,
            );
            router::init(Destination::Home, cx);

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None, FIRST_SIZE, cx,
                    ))),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_rem_size(cx.theme().font_size);
                    cx.new(Page::new)
                },
            )
            .expect("cannot open the window");

            cx.activate(true);
        });
}
