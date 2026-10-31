//! A miniature of the Now playing view: the ambient field, artwork, title and artist.
//! It takes no pointer input and draws no visualizer, so it needs no spectrum or animation drive.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, Window, div};
use ui::{ActiveTheme as _, Artwork, Text};

use crate::screens::settings::{Layout, SettingsView};

/// How tall the miniature stands for its width, the shape of a window.
const ASPECT: f32 = 0.5;
/// How much of the miniature's height the cover takes.
const COVER: f32 = 0.46;
/// How much of the miniature's width the title and artist may take.
const META: f32 = 0.6;

/// The Now playing page's preview, a miniature of the Now playing view for the track playing,
/// or for one from history while nothing is loaded.
pub(in crate::screens::settings) fn preview(
    view: &SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Option<AnyElement> {
    let width = Layout::of(window, cx).width;
    let height = (width * ASPECT).round();
    let sample = view.previews.playing_or_picked(cx);
    let cover = sample.large_cover(cx);
    let ambient = view.settings.read(cx).ambient();

    let field = ambient.then(|| view.previews.ambient(cover.clone(), cx));
    // over the field the miniature wears what fullscreen would, not the theme around it
    let theme = match &field {
        Some(field) => field.read(cx).theme(cx),
        None => *cx.theme(),
    };
    let side = (height * COVER).round();

    Some(
        super::framed(
            div()
                .relative()
                .w_full()
                .h(height)
                .overflow_hidden()
                .bg(theme.background)
                .text_color(theme.foreground)
                .children(field)
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .child(Artwork::new(cover).size(side).corner_radius(theme.radius))
                        .child(
                            div()
                                .mt_2()
                                .max_w(width * META)
                                .truncate()
                                .text_size(theme.text(Text::Body))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(sample.title.clone()),
                        )
                        .child(
                            div()
                                .max_w(width * META)
                                .truncate()
                                .text_size(theme.text(Text::Small))
                                .text_color(theme.muted_foreground)
                                .child(sample.artist.clone()),
                        ),
                ),
            cx,
        )
        .into_any_element(),
    )
}
