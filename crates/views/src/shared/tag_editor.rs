use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{App, Context, Div, Entity, FocusHandle, Global, Render, SharedString, Window, div};
use i18n::t;
use music::{Track, TrackTags};
use state::{Io, Sonora, TagState, Tags};
use ui::{ActiveTheme as _, Button, Dismiss, FORM_CONTEXT, Input, Modal, Submit, TabBar, Text};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sheet {
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

/// The metadata editor for one local track. Single-value fields each keep one input, and the
/// artists keep one input per name so a track can credit several.
pub(crate) struct TagEditor {
    tags: Entity<Tags>,
    inputs: Vec<Entity<Input>>,
    /// One input per artist, never empty, so a track with no artist still shows a field.
    artists: Vec<Entity<Input>>,
    sheet: Sheet,
    filled: bool,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
}

struct Installed(Entity<TagEditor>);

impl Global for Installed {}

impl TagEditor {
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let sonora = Sonora::global(cx);
            let session = sonora.session.clone();
            let library = sonora.library.clone();
            let editor = cx.new(|cx| {
                let io = Io::global(cx);
                let tags = cx.new(|cx| Tags::new(session, library, io, cx));
                cx.observe(&tags, |_, _, cx| cx.notify()).detach();
                let inputs = Field::ALL
                    .map(|field| cx.new(|cx| Input::new(field.key(), cx)))
                    .to_vec();

                Self {
                    tags,
                    inputs,
                    artists: vec![artist_input(cx)],
                    sheet: Sheet::Song,
                    filled: false,
                    focus: cx.focus_handle(),
                    restore: None,
                }
            });
            cx.set_global(Installed(editor));
        }
        cx.global::<Installed>().0.clone()
    }

    pub fn open(track: Track, window: &mut Window, cx: &mut App) {
        let editor = Self::entity(cx);
        editor.update(cx, |this, cx| this.show(track, window, cx));
    }

    fn show(&mut self, track: Track, window: &mut Window, cx: &mut Context<Self>) {
        self.restore = window.focused(cx);
        self.sheet = Sheet::Song;
        self.filled = false;
        window.focus(&self.focus, cx);
        self.tags.update(cx, |tags, cx| tags.open(track, cx));
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tags.update(cx, |tags, cx| tags.close(cx));
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn fill(&mut self, tags: &TrackTags, cx: &mut Context<Self>) {
        for field in Field::ALL {
            let value = field.read(tags).to_owned();
            self.inputs[field.slot()].update(cx, |input, cx| input.set_text(value, cx));
        }
        let names = match tags.artists.is_empty() {
            true => vec![String::new()],
            false => tags.artists.clone(),
        };
        self.artists = names
            .into_iter()
            .map(|name| {
                let input = artist_input(cx);
                input.update(cx, |input, cx| input.set_text(name, cx));
                input
            })
            .collect();
        self.filled = true;
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        let TagState::Ready(held) = self.tags.read(cx).state() else {
            return;
        };
        let mut edited = held.as_ref().clone();
        for field in Field::ALL {
            let value = self.inputs[field.slot()].read(cx).text().trim().to_owned();
            field.write(&mut edited, value);
        }
        edited.artists = self
            .artists
            .iter()
            .map(|input| input.read(cx).text().trim().to_owned())
            .filter(|name| !name.is_empty())
            .collect();
        self.tags.update(cx, |tags, cx| tags.save(edited, cx));
        cx.notify();
    }

    /// Adds an empty artist field below the others and puts the caret in it.
    fn add_artist(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = artist_input(cx);
        input.update(cx, |input, cx| input.focus(window, cx));
        self.artists.push(input);
        cx.notify();
    }

    /// Drops the artist field at `slot`, keeping the last one so the list never goes empty.
    fn remove_artist(&mut self, slot: usize, cx: &mut Context<Self>) {
        if self.artists.len() > 1 && slot < self.artists.len() {
            self.artists.remove(slot);
            cx.notify();
        }
    }

    fn sheets(&self, cx: &Context<Self>) -> impl IntoElement {
        div().flex().child(self.bar(cx))
    }

    fn bar(&self, cx: &Context<Self>) -> TabBar {
        TabBar::new("tag-editor-sheets")
            .name(t!("tags-edit-title"))
            .items(Sheet::ALL.map(|sheet| {
                Button::new(sheet.key())
                    .label(i18n::lookup(sheet.key(), None))
                    .small()
                    .ghost()
                    .selected(self.sheet == sheet)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sheet = sheet;
                        Modal::rewind("tag-editor", cx);
                        cx.notify();
                    }))
            }))
    }

    fn rows(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(self.sheet.rows().iter().map(|row| {
                match row {
                    Row::Fields(fields) => div()
                        .flex()
                        .gap_3()
                        .children(fields.iter().map(|field| {
                            labelled(field.key(), cx)
                                .flex_1()
                                .min_w_0()
                                .child(self.inputs[field.slot()].clone())
                        }))
                        .into_any_element(),
                    Row::Artists => self.artist_rows(cx).into_any_element(),
                }
            }))
    }

    /// The artist fields, one per name, with a button to add another beside the label and one to
    /// remove each field while there is more than one.
    fn artist_rows(&self, cx: &Context<Self>) -> impl IntoElement {
        let removable = self.artists.len() > 1;
        let theme = *cx.theme();

        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.muted_foreground)
                            .child(t!("tags-artists")),
                    )
                    .child(
                        Button::new("tag-artist-add")
                            .icon("icons/plus.svg")
                            .tooltip("tags-add-artist")
                            .small()
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_artist(window, cx)),
                            ),
                    ),
            )
            .children(self.artists.iter().enumerate().map(|(slot, input)| {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().flex_1().min_w_0().child(input.clone()))
                    .when(removable, |row| {
                        row.child(
                            Button::new(("tag-artist-remove", slot))
                                .icon("icons/x.svg")
                                .tooltip("tags-remove-artist")
                                .small()
                                .ghost()
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.remove_artist(slot, cx)),
                                ),
                        )
                    })
            }))
    }
}

impl Render for TagEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tags = self.tags.read(cx);
        let Some(track) = tags.track().cloned() else {
            if let Some(focus) = self.restore.take() {
                window.focus(&focus, cx);
            }
            return div().into_any_element();
        };
        let theme = *cx.theme();
        let saving = tags.saving();
        let ready = matches!(tags.state(), TagState::Ready(_));
        let trouble = match tags.state() {
            TagState::Failed(reason) => Some(reason.clone()),
            _ => None,
        };
        let pending = match (ready, self.filled) {
            (true, false) => match tags.state() {
                TagState::Ready(tags) => Some(tags.as_ref().clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(pending) = pending {
            self.fill(&pending, cx);
        }

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, _, cx| {
                cx.stop_propagation();
                this.apply(cx);
            }))
            .child(
                Modal::new("tag-editor", t!("tags-edit-title"))
                    .w(theme.metrics.cover * 3.2)
                    .h(theme.metrics.cover * 3.6)
                    .detail(SharedString::from(track.name.clone()))
                    .when(ready, |modal| {
                        modal.child(self.sheets(cx)).child(self.rows(cx))
                    })
                    .when_some(trouble, |modal, trouble| {
                        let trouble = SharedString::from(trouble);
                        modal.child(
                            div()
                                .id("tag-editor-trouble")
                                .role(Role::Alert)
                                .aria_label(trouble.clone())
                                .text_size(theme.text(Text::Small))
                                .text_color(theme.danger)
                                .child(trouble),
                        )
                    })
                    .action(
                        Button::new("cancel-tag-edit")
                            .ghost()
                            .label(t!("common-cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    )
                    .action(
                        Button::new("save-tag-edit")
                            .primary()
                            .label(t!("common-save"))
                            .disabled(!ready || saving)
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}

fn artist_input(cx: &mut Context<TagEditor>) -> Entity<Input> {
    cx.new(|cx| Input::new("tags-artist", cx))
}

/// A field's label above whatever the caller adds below it.
fn labelled(key: &'static str, cx: &App) -> Div {
    let theme = cx.theme();
    div().flex().flex_col().gap_1().child(
        div()
            .text_size(theme.text(Text::Small))
            .text_color(theme.muted_foreground)
            .child(i18n::lookup(key, None)),
    )
}
