use std::borrow::Cow;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Entity, EntityId, Pixels, ScrollHandle, SharedString, Window,
    WindowBounds, WindowOptions, div, px, size, svg,
};
use ui::{
    ActiveTheme as _, Look, Rounding, RowSize, Scrollbar, Scroller, Text, Theme, ThemeKind, eyebrow,
};

const INTER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/Inter-Regular.ttf"
));

const FIRST_SIZE: gpui::Size<Pixels> = size(px(900.), px(720.));
const GLYPH: Pixels = px(22.);
const NAME: Pixels = px(190.);
const CELL: Pixels = px(120.);
const ROW: Pixels = px(44.);

struct Gallery {
    scrollbar: Entity<Scrollbar>,
}

impl Gallery {
    fn new(id: EntityId, cx: &mut Context<Self>) -> Self {
        Self {
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id)),
        }
    }

    fn header(&self, cx: &App) -> impl IntoElement {
        let theme = *cx.theme();

        div()
            .flex()
            .items_end()
            .px_6()
            .py_3()
            .gap_4()
            .border_b_1()
            .border_color(theme.table_row_border)
            .child(div().w(NAME).flex_none().child(eyebrow("icon", cx)))
            .children(icons::packs().map(|pack| {
                div()
                    .w(CELL)
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(pack.title())
                    .child(
                        div()
                            .text_size(theme.text(Text::Tiny))
                            .text_color(theme.muted_foreground)
                            .child(SharedString::from(format!("{} icons", pack.icons().len()))),
                    )
            }))
    }

    fn row(&self, name: &'static str, cx: &App) -> impl IntoElement {
        let theme = *cx.theme();

        div()
            .flex()
            .items_center()
            .h(ROW)
            .px_6()
            .gap_4()
            .border_b_1()
            .border_color(theme.table_row_border)
            .child(
                div()
                    .w(NAME)
                    .flex_none()
                    .truncate()
                    .text_size(theme.text(Text::Small))
                    .child(name),
            )
            .children(icons::packs().map(|pack| {
                let own = pack.icon(name).is_some();
                div().w(CELL).flex_none().child(
                    svg()
                        .path(icons::shown(pack, name))
                        .size(GLYPH)
                        .flex_none()
                        .text_color(match own {
                            true => theme.foreground,
                            false => theme.muted_foreground.opacity(0.4),
                        }),
                )
            }))
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let rows: Vec<_> = icons::NAMES.iter().map(|name| self.row(name, cx)).collect();

        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family("Inter")
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_size(theme.text(Text::Body))
            .child(self.header(cx))
            .child(
                Scroller::new("gallery", &self.scrollbar)
                    .flex_1()
                    .min_h_0()
                    .child(div().flex().flex_col().children(rows)),
            )
            .child(
                div()
                    .px_6()
                    .py_2()
                    .text_size(theme.text(Text::Tiny))
                    .text_color(theme.muted_foreground)
                    .child("faded glyphs are borrowed from the base pack"),
            )
    }
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

            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None, FIRST_SIZE, cx,
                    ))),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_rem_size(cx.theme().font_size);
                    cx.new(|cx| Gallery::new(cx.entity_id(), cx))
                },
            )
            .expect("cannot open the window");

            cx.activate(true);
        });
}
