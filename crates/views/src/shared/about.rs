//! The artist's About card on an artist page, and the dialog it opens with the whole biography.

use std::rc::Rc;
use ui::Smooth as _;

use i18n::t;
use router::Destination;
use rune::{Cursor, El, ScrollSpec, Span, col, rich, row};
use ui::{Avatar, Button, Modal, Text, Weight};

use crate::Ui;
use crate::shared::dialogs::rems;

/// The portrait's side.
const PORTRAIT: f32 = 88.;
/// How many lines of the biography the card shows.
const LINES: usize = 3;
/// How wide the dialog is, in covers.
const DIALOG: f32 = 4.5;
/// How tall the biography gets in the dialog before it scrolls, and how far its edges fade.
const BIO_HEIGHT: f32 = 360.;
const FADE: f32 = 64.;
const BORDER: f32 = 1.;

type Open = Rc<dyn Fn(&mut Ui)>;

/// The About dialog while it is up: whose biography, and the artist page it can go to.
pub(crate) struct About {
    name: String,
    biography: Option<String>,
    artist: Option<String>,
}

/// Opens the About dialog on `name`'s `biography`, with a button to `artist`'s page when given.
pub(crate) fn open(ui: &mut Ui, name: String, biography: Option<String>, artist: Option<String>) {
    ui.dialogs.about = Some(About {
        name,
        biography: biography.filter(|biography| !biography.is_empty()),
        artist,
    });
}

fn close(ui: &mut Ui) {
    ui.dialogs.about = None;
}

/// The About card: the portrait, the artist's name and the first lines of the biography.
pub(crate) struct AboutArtist {
    key: String,
    name: String,
    cover: Option<String>,
    biography: Option<String>,
    on_open: Option<Open>,
}

impl AboutArtist {
    pub(crate) fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            cover: None,
            biography: None,
            on_open: None,
        }
    }

    pub(crate) fn cover(mut self, cover: Option<String>) -> Self {
        self.cover = cover;
        self
    }

    pub(crate) fn biography(mut self, biography: Option<String>) -> Self {
        self.biography = biography;
        self
    }

    /// What a press on the card does, usually [`open`]. Without it the card is a plain group.
    pub(crate) fn on_open(mut self, f: impl Fn(&mut Ui) + 'static) -> Self {
        self.on_open = Some(Rc::new(f));
        self
    }

    pub(crate) fn build(self, ui: &Ui) -> El {
        let theme = &ui.theme;
        let about = blurb(self.biography);
        let pressable = self.on_open.is_some();
        let words = col()
            .grow(1.)
            .shrink()
            .min_w(0.)
            .gap(rems(theme, 0.5))
            .child(ui::eyebrow(t!("artist-about"), theme).a11y_hidden())
            .child(
                ui::heading(self.name.clone(), theme)
                    .truncate()
                    .a11y_hidden(),
            )
            .child(
                rune::text(about.clone())
                    .font(ui::font(theme, Text::Body, Weight::NORMAL))
                    .color(theme.muted_foreground.color())
                    .max_lines(LINES)
                    .min_w(0.)
                    .a11y_hidden(),
            );
        let mut card = row()
            .key(self.key)
            // The card opens the artist only when it has somewhere to open, so it reads as a
            // button only then and as a named group otherwise.
            .role(match pressable {
                true => "button",
                false => "group",
            })
            .label(self.name)
            .description(about)
            .w_full()
            .center_y()
            .gap(rems(theme, 1.25))
            .padding(rems(theme, 1.25))
            .radius(theme.radius)
            .border(BORDER, theme.border.color())
            .child(
                Avatar::new(ui.artwork(), self.cover.as_deref())
                    .size(PORTRAIT)
                    .build(theme),
            )
            .child(words);
        if let Some(open) = self.on_open {
            card = card
                .focusable(true)
                .cursor(Cursor::Pointer)
                .hover(theme.secondary.color())
                .press_scale(ui::PRESS_SCALE_CARD)
                .on_press(move |ui: &mut Ui| open(ui));
        }
        card
    }
}

/// Mounts the About dialog, if it is up, into `layer`.
pub(crate) fn view(ui: &Ui, layer: &mut Vec<El>) {
    let Some(about) = &ui.dialogs.about else {
        return;
    };
    let theme = &ui.theme;
    let muted = theme.muted_foreground.color();
    let style = ui::font(theme, Text::Body, Weight::NORMAL);
    let body = match &about.biography {
        Some(biography) => {
            let (plain, italics) = italicised(biography);
            col()
                .key("artist-about-bio")
                .w_full()
                .max_h(BIO_HEIGHT)
                .min_h(0.)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .smooth::<Ui>("artist-about-bio")
                .scroll_edge_fade(FADE)
                .child(
                    rich(plain.clone(), italics)
                        .key("artist-about-bio-text")
                        .role("paragraph")
                        .label(plain)
                        .font(style)
                        .color(muted)
                        .wrap(),
                )
        }
        None => {
            let fallback = t!("artist-about-fallback").into_owned();
            rune::text(fallback.clone())
                .key("artist-about-bio-text")
                .role("paragraph")
                .label(fallback)
                .font(style)
                .color(muted)
                .wrap()
        }
    };
    let mut modal = Modal::new("artist-about-dialog", t!("artist-about"))
        .width(theme.metrics.cover * DIALOG)
        .detail(about.name.clone())
        .child(body);
    if let Some(artist) = about.artist.clone() {
        modal = modal.action(
            Button::new("artist-about-open")
                .label(t!("artist-about-open"))
                .outline()
                .on_press(move |ui: &mut Ui| {
                    close(ui);
                    ui.go(Destination::Artist(artist.clone().into()));
                })
                .build(theme),
        );
    }
    modal.on_dismiss(close).build(theme, layer);
}

/// The text the card shows: the biography without its tags, or the fallback line.
fn blurb(biography: Option<String>) -> String {
    biography
        .filter(|biography| !biography.is_empty())
        .map(|biography| italicised(&biography).0)
        .unwrap_or_else(|| t!("artist-about-fallback").into_owned())
}

/// Apple's biographies mark titles with `<i>` italics. Strips the tags and returns the plain
/// text with the spans to draw slanted. A tag without its partner, or one this does not know,
/// is dropped rather than shown.
fn italicised(biography: &str) -> (String, Vec<Span>) {
    let mut plain = String::with_capacity(biography.len());
    let mut italics = Vec::new();
    let mut rest = biography;
    let mut open: Option<usize> = None;
    while let Some(at) = rest.find('<') {
        plain.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(end) = after.find('>') else {
            plain.push_str(after);
            rest = "";
            break;
        };
        match &after[..=end] {
            "<i>" => open = Some(plain.len()),
            "</i>" => {
                if let Some(start) = open.take()
                    && start < plain.len()
                {
                    italics.push(Span {
                        italic: true,
                        ..Span::new(start, plain.len())
                    });
                }
            }
            _ => {}
        }
        rest = &after[end + 1..];
    }
    plain.push_str(rest);
    (plain, italics)
}
