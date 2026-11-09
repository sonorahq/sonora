//! The metadata editor for one local track: its tags in three sheets (the song, the album and
//! the details), each a set of fields, with one field per credited artist.

use i18n::t;
use music::{Track, TrackTags};
use rune::{El, TextEdit, Tx, col, row, text};
use state::{TagState, Tags};
use ui::{Button, Input, Modal, TabBar, Text, Weight};

use crate::Ui;
use crate::shared::dialogs::rems;

/// How wide the dialog is, in covers.
const WIDTH: f32 = 3.2;
/// How tall the sheets' area is at least, in covers, so switching sheets keeps the dialog's size.
const SHEET_HEIGHT: f32 = 2.4;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Sheet {
    #[default]
    Song,
    Album,
    Details,
}

impl Sheet {
    const ALL: [Self; 3] = [Self::Song, Self::Album, Self::Details];

    fn key(self) -> &'static str {
        match self {
            Sheet::Song => "tags-sheet-song",
            Sheet::Album => "tags-sheet-album",
            Sheet::Details => "tags-sheet-details",
        }
    }

    fn rows(self) -> &'static [Row] {
        match self {
            Sheet::Song => &[
                Row::Fields(&[Field::Title]),
                Row::Artists,
                Row::Fields(&[Field::TrackNumber, Field::TrackTotal]),
                Row::Fields(&[Field::DiscNumber, Field::DiscTotal]),
            ],
            Sheet::Album => &[
                Row::Fields(&[Field::Album]),
                Row::Fields(&[Field::AlbumArtist]),
                Row::Fields(&[Field::Year, Field::Genre]),
            ],
            Sheet::Details => &[
                Row::Fields(&[Field::Composer]),
                Row::Fields(&[Field::Publisher]),
                Row::Fields(&[Field::Isrc]),
                Row::Fields(&[Field::Comment]),
            ],
        }
    }
}

/// One line of a sheet: single-value fields side by side, or the list of the track's artists.
#[derive(Clone, Copy)]
enum Row {
    Fields(&'static [Field]),
    Artists,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    TrackNumber,
    TrackTotal,
    DiscNumber,
    DiscTotal,
    Album,
    AlbumArtist,
    Year,
    Genre,
    Composer,
    Publisher,
    Isrc,
    Comment,
}

impl Field {
    const ALL: [Self; 13] = [
        Self::Title,
        Self::TrackNumber,
        Self::TrackTotal,
        Self::DiscNumber,
        Self::DiscTotal,
        Self::Album,
        Self::AlbumArtist,
        Self::Year,
        Self::Genre,
        Self::Composer,
        Self::Publisher,
        Self::Isrc,
        Self::Comment,
    ];

    fn slot(self) -> usize {
        Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or_default()
    }

    fn key(self) -> &'static str {
        match self {
            Field::Title => "tags-title",
            Field::TrackNumber => "tags-track",
            Field::TrackTotal => "tags-track-total",
            Field::DiscNumber => "tags-disc",
            Field::DiscTotal => "tags-disc-total",
            Field::Album => "tags-album",
            Field::AlbumArtist => "tags-album-artist",
            Field::Year => "tags-year",
            Field::Genre => "tags-genre",
            Field::Composer => "tags-composer",
            Field::Publisher => "tags-publisher",
            Field::Isrc => "tags-isrc",
            Field::Comment => "tags-comment",
        }
    }

    fn read(self, tags: &TrackTags) -> &str {
        match self {
            Field::Title => &tags.title,
            Field::TrackNumber => &tags.track_number,
            Field::TrackTotal => &tags.track_total,
            Field::DiscNumber => &tags.disc_number,
            Field::DiscTotal => &tags.disc_total,
            Field::Album => &tags.album,
            Field::AlbumArtist => &tags.album_artist,
            Field::Year => &tags.year,
            Field::Genre => &tags.genre,
            Field::Composer => &tags.composer,
            Field::Publisher => &tags.publisher,
            Field::Isrc => &tags.isrc,
            Field::Comment => &tags.comment,
        }
    }

    fn write(self, tags: &mut TrackTags, value: String) {
        let held = match self {
            Field::Title => &mut tags.title,
            Field::TrackNumber => &mut tags.track_number,
            Field::TrackTotal => &mut tags.track_total,
            Field::DiscNumber => &mut tags.disc_number,
            Field::DiscTotal => &mut tags.disc_total,
            Field::Album => &mut tags.album,
            Field::AlbumArtist => &mut tags.album_artist,
            Field::Year => &mut tags.year,
            Field::Genre => &mut tags.genre,
            Field::Composer => &mut tags.composer,
            Field::Publisher => &mut tags.publisher,
            Field::Isrc => &mut tags.isrc,
            Field::Comment => &mut tags.comment,
        };
        *held = value;
    }
}

/// The editor's fields. The tags model (`store.pages.tags`) reads the file; the fields take its
/// values the first time anything touches them, so they never fill from inside the view.
#[derive(Default)]
pub(crate) struct Editor {
    fields: Vec<TextEdit>,
    /// One field per artist, never empty once filled, so a track with no artist still shows one.
    artists: Vec<TextEdit>,
    sheet: Sheet,
    filled: bool,
}

/// Opens the editor on `track` and starts reading its tags.
pub(crate) fn open(ui: &mut Ui, track: Track) {
    ui.dialogs.tags = Editor::default();
    let store = &mut ui.store;
    let tags = store.pages.tags.insert(Tags::new());
    tags.open(track, &store.session, &mut store.cx);
}

/// Whether the editor is up: it is from the moment it opens until a save lands or it closes.
pub(crate) fn shows(ui: &Ui) -> bool {
    ui.store
        .pages
        .tags
        .as_ref()
        .is_some_and(|tags| tags.track().is_some())
}

/// Puts the editor away, dropping a read still under way.
pub(crate) fn close(ui: &mut Ui) {
    ui.store.pages.tags = None;
    ui.dialogs.tags = Editor::default();
}

/// The tags as read from the file, once they are.
fn ready(ui: &Ui) -> Option<&TrackTags> {
    match ui.store.pages.tags.as_ref()?.state() {
        TagState::Ready(tags) => Some(tags),
        _ => None,
    }
}

/// Fills the fields from the file's tags, once they are read and before anything edits them.
fn fill(ui: &mut Ui) {
    if ui.dialogs.tags.filled {
        return;
    }
    let Some(tags) = ready(ui) else {
        return;
    };
    let fields = Field::ALL
        .map(|field| TextEdit::new(field.read(tags)))
        .to_vec();
    let artists = match tags.artists.is_empty() {
        true => vec![TextEdit::default()],
        false => tags.artists.iter().map(TextEdit::new).collect(),
    };
    let editor = &mut ui.dialogs.tags;
    editor.fields = fields;
    editor.artists = artists;
    editor.filled = true;
}

/// Writes the fields back to the file.
fn apply(ui: &mut Ui) {
    fill(ui);
    let Some(held) = ready(ui) else {
        return;
    };
    let mut edited = held.clone();
    let editor = &ui.dialogs.tags;
    for field in Field::ALL {
        let value = editor.fields[field.slot()].text.trim().to_owned();
        field.write(&mut edited, value);
    }
    edited.artists = editor
        .artists
        .iter()
        .map(|edit| edit.text.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    let store = &mut ui.store;
    if let Some(tags) = store.pages.tags.as_mut() {
        tags.save(edited, &store.session, &mut store.cx);
    }
    ui.settle();
}

/// Adds an empty artist field below the others and puts the caret in it.
fn add_artist(ui: &mut Ui) {
    fill(ui);
    let mut edit = TextEdit::default();
    edit.focused = true;
    ui.dialogs.tags.artists.push(edit);
}

/// Drops the artist field at `slot`, keeping the last one so the list never goes empty.
fn remove_artist(ui: &mut Ui, slot: usize) {
    fill(ui);
    let artists = &mut ui.dialogs.tags.artists;
    if artists.len() > 1 && slot < artists.len() {
        artists.remove(slot);
    }
}

/// The field for `field`, showing the file's value until the fields are filled.
fn field_input(ui: &Ui, tags: &TrackTags, field: Field) -> El {
    let editor = &ui.dialogs.tags;
    let slot = field.slot();
    let shown = match editor.filled {
        true => editor.fields[slot].clone(),
        false => TextEdit::new(field.read(tags)),
    };
    Input::new(field.key(), &shown, String::new(), move |ui: &mut Ui| {
        fill(ui);
        &mut ui.dialogs.tags.fields[slot]
    })
    .name(i18n::lookup(field.key(), None))
    .on_submit(|ui: &mut Ui, _: &mut Tx| apply(ui))
    .build(&ui.theme)
    .w_full()
}

/// The sheet tabs over the fields.
fn bar(ui: &Ui) -> El {
    let sheet = ui.dialogs.tags.sheet;
    row().key("tag-editor-tabs").child(
        TabBar::new("tag-editor-sheets")
            .name(t!("tags-edit-title"))
            .items(Sheet::ALL.map(|choice| {
                Button::new(choice.key())
                    .label(i18n::lookup(choice.key(), None))
                    .small()
                    .ghost()
                    .selected(sheet == choice)
                    .on_press(move |ui: &mut Ui| ui.dialogs.tags.sheet = choice)
            }))
            .build(&ui.theme),
    )
}

/// The rows of the sheet on show.
fn rows(ui: &Ui, tags: &TrackTags) -> El {
    let theme = &ui.theme;
    let sheet = ui.dialogs.tags.sheet;
    col()
        .key(format!("tag-editor-{}", sheet.key()))
        .w_full()
        .min_h(theme.metrics.cover * SHEET_HEIGHT)
        .gap(rems(theme, 0.75))
        .children(sheet.rows().iter().map(|line| match line {
            Row::Fields(fields) => row().w_full().gap(rems(theme, 0.75)).children(
                fields.iter().map(|field| {
                    labelled(ui, field.key())
                        .grow(1.)
                        .min_w(0.)
                        .child(field_input(ui, tags, *field))
                }),
            ),
            Row::Artists => artist_rows(ui, tags),
        }))
}

/// The artist fields, one per name, with a button to add another beside the label and one to
/// remove each field while there is more than one.
fn artist_rows(ui: &Ui, tags: &TrackTags) -> El {
    let theme = &ui.theme;
    let editor = &ui.dialogs.tags;
    let names: Vec<TextEdit> = match editor.filled {
        true => editor.artists.clone(),
        false => match tags.artists.is_empty() {
            true => vec![TextEdit::default()],
            false => tags.artists.iter().map(TextEdit::new).collect(),
        },
    };
    let removable = names.len() > 1;
    let count = names.len();
    let head = row()
        .key("tag-artists-head")
        .w_full()
        .center_y()
        .justify_between()
        .child(caption(ui, t!("tags-artists").into_owned()))
        .child(
            Button::new("tag-artist-add")
                .icon("icons/plus.svg")
                .tooltip(t!("tags-add-artist"))
                .small()
                .ghost()
                .on_press(add_artist)
                .build(theme),
        );
    col()
        .key("tag-artists")
        .w_full()
        .gap(rems(theme, 0.25))
        .child(head)
        .children(names.iter().enumerate().map(|(slot, name)| {
            let field = Input::new(
                format!("tags-artist-{slot}"),
                name,
                String::new(),
                move |ui: &mut Ui| {
                    fill(ui);
                    &mut ui.dialogs.tags.artists[slot]
                },
            )
            .name(t!("tags-artist"))
            .on_submit(|ui: &mut Ui, _: &mut Tx| apply(ui))
            .build(theme)
            .grow(1.)
            .min_w(0.)
            .position_in_set(slot + 1, count);
            let mut line = row()
                .key(format!("tag-artist-row-{slot}"))
                .w_full()
                .center_y()
                .gap(rems(theme, 0.25))
                .transition(rune::Transition::opacity())
                .child(field);
            if removable {
                line = line.child(
                    Button::new(format!("tag-artist-remove-{slot}"))
                        .icon("icons/x.svg")
                        .tooltip(t!("tags-remove-artist"))
                        .small()
                        .ghost()
                        .on_press(move |ui: &mut Ui| remove_artist(ui, slot))
                        .build(theme),
                );
            }
            line
        }))
}

/// A field's label above whatever the caller adds below it.
fn labelled(ui: &Ui, key: &'static str) -> El {
    col()
        .gap(rems(&ui.theme, 0.25))
        .child(caption(ui, i18n::lookup(key, None).into_owned()))
}

/// Small muted text over a field. The field reads its own name, so this stays out of the tree.
fn caption(ui: &Ui, label: String) -> El {
    let theme = &ui.theme;
    text(label)
        .font(ui::font(theme, Text::Small, Weight::NORMAL))
        .color(theme.muted_foreground.color())
        .a11y_hidden()
}

/// Mounts the editor, if it is up, into `layer`.
pub(crate) fn view(ui: &Ui, layer: &mut Vec<El>) {
    let Some(tags) = ui.store.pages.tags.as_ref() else {
        return;
    };
    let Some(track) = tags.track() else {
        return;
    };
    let theme = &ui.theme;
    let saving = tags.saving();
    let read = ready(ui);
    let trouble = match tags.state() {
        TagState::Failed(reason) => Some(reason.clone()),
        _ => None,
    };
    let mut modal = Modal::new("tag-editor", t!("tags-edit-title"))
        .width(theme.metrics.cover * WIDTH)
        .detail(track.name.clone());
    if let Some(read) = read {
        modal = modal.child(bar(ui)).child(rows(ui, read));
    }
    if let Some(trouble) = trouble {
        modal = modal.child(
            text(trouble.clone())
                .key("tag-editor-trouble")
                .role("alert")
                .label(trouble)
                .font(ui::font(theme, Text::Small, Weight::NORMAL))
                .color(theme.danger.color())
                .wrap(),
        );
    }
    modal
        .action(
            Button::new("cancel-tag-edit")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(close)
                .build(theme),
        )
        .action(
            Button::new("save-tag-edit")
                .primary()
                .label(t!("common-save"))
                .disabled(read.is_none() || saving)
                .loading(saving)
                .on_press(apply)
                .build(theme)
                .default_button(),
        )
        .on_dismiss(close)
        .build(theme, layer);
}
