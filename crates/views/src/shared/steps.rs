//! A numbered list of instructions, as the cookie sign-in walks through.

use rune::{El, TextStyle, col, row, text};
use ui::{Text, Theme, Weight};

/// How large a step's number badge stands.
const BADGE: f32 = 28.;
const BORDER: f32 = 1.;
/// How long each step waits after the one before it to slide in, in seconds.
const STAGGER: f32 = 0.04;

/// A numbered list of instructions. Screen readers meet it as a list whose items carry the
/// step's text and their place in it. Each step slides in a beat after the one before it.
pub(crate) fn steps(items: impl IntoIterator<Item = impl Into<String>>, theme: &Theme) -> El {
    let items: Vec<String> = items.into_iter().map(Into::into).collect();
    let total = items.len();
    let pad = theme.metrics.pad;
    col()
        .key("steps")
        .role("list")
        .w_full()
        .gap(theme.font_size * 0.5)
        .children(items.into_iter().enumerate().map(|(i, step)| {
            row()
                .key(format!("step-{i}"))
                .role("listitem")
                .label(step.clone())
                .position_in_set(i + 1, total)
                .center_y()
                .gap(theme.font_size * 0.75)
                .w_full()
                .min_w(0.)
                .radius(theme.radius)
                .padding(pad / 2.)
                .transition(
                    rune::Transition::offset(0.0, 6.0).combined(rune::Transition::opacity()),
                )
                .enter_delay(STAGGER * i as f32)
                .child(
                    row()
                        .size(BADGE, BADGE)
                        .center()
                        .radius(BADGE / 2.)
                        .border(BORDER, theme.border.color())
                        .fill(theme.secondary.color())
                        .a11y_hidden()
                        .child(
                            text((i + 1).to_string())
                                .font(TextStyle::new(theme.text(Text::Small), Weight::SEMIBOLD))
                                .color(theme.foreground.color()),
                        ),
                )
                .child(
                    text(step)
                        .grow(1.)
                        .shrink()
                        .min_w(0.)
                        .wrap()
                        .font(TextStyle::new(theme.text(Text::Small), Weight::NORMAL))
                        .color(theme.muted_foreground.color())
                        .a11y_hidden(),
                )
        }))
}
