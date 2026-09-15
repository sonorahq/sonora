use gpui::prelude::*;
use gpui::{Context, Entity, Pixels, Point, Render, ScrollHandle, SharedString, Window, div, px};
use i18n::t;
use music::Playlist;
use router::{Destination, navigate};
use state::{FolderRow, FolderShuffle, Library, Playback, Shelf};
use ui::{ActiveTheme as _, Button, Popup, Scrollbar, Scroller, Vacancy};

use crate::chrome::Chrome;
use crate::shared::album_grid::CardGrid;
use crate::shared::cards;
use crate::shared::cells;
use crate::shared::hero::{HeroMetaStrip, PageHero};
use crate::shared::menus::{folder_menu, playlist_menu};

const FOLDER: &str = "icons/folder.svg";
const STEADY: Pixels = px(0.5);

/// What the context menu of the page was summoned on.
#[derive(Clone)]
enum Summoned {
    Playlist(Playlist),
    Folder(FolderRow),
}

/// One folder of playlists: what sits directly inside it, and a shuffle over everything below it.
/// The folder comes from the library's outline, so the page needs nothing loaded of its own.
pub(crate) struct FolderView {
    library: Entity<Library>,
    playback: Entity<Playback>,
    shuffle: Entity<FolderShuffle>,
    scrollbar: Entity<Scrollbar>,
    id: Option<SharedString>,
    width: Pixels,
    context_menu: Option<(Summoned, Point<Pixels>)>,
}

impl FolderView {
    pub(crate) fn new(
        library: Entity<Library>,
        playback: Entity<Playback>,
        shuffle: Entity<FolderShuffle>,
        cx: &mut Context<Self>,
    ) -> Self {
        let id = cx.entity_id();

        cx.observe(&library, |_, _, cx| cx.notify()).detach();
        cx.observe(&playback, |_, _, cx| cx.notify()).detach();
        cx.observe(&shuffle, |_, _, cx| cx.notify()).detach();

        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();

        Self {
            library,
            playback,
            shuffle,
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id)),
            id: None,
            width: Pixels::ZERO,
            context_menu: None,
        }
    }

    pub(crate) fn open(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.id.as_deref() == Some(id) {
            return;
        }
        self.id = Some(SharedString::from(id.to_owned()));
        self.context_menu = None;
        self.scrollbar
            .read(cx)
            .scroll()
            .set_offset(gpui::Point::default());
        cx.notify();
    }

    fn shelf(&self) -> Shelf {
        Shelf::of(self.id.as_deref().unwrap_or_default())
    }

    fn folder(&self, cx: &Context<Self>) -> Option<FolderRow> {
        let id = self.id.as_deref()?;
        self.library
            .read(cx)
            .state(self.shelf())
            .outline()
            .folder(id)
    }

    /// The folders and playlists directly inside, folders first, as a page of cards.
    fn contents(&self, cx: &Context<Self>) -> (Vec<FolderRow>, Vec<Playlist>) {
        let Some(id) = self.id.as_deref() else {
            return (Vec::new(), Vec::new());
        };
        let held = self.library.read(cx).state(self.shelf());
        let outline = held.outline();
        let playlists = held.playlists();

        let mut folders = Vec::new();
        let mut listed = Vec::new();
        for &at in outline.level(Some(id)) {
            match outline.folder_at(at) {
                Some(folder) => folders.push(folder),
                None => listed.extend(
                    outline
                        .playlist_at(at)
                        .and_then(|index| playlists.get(index))
                        .cloned(),
                ),
            }
        }

        (folders, listed)
    }

    fn header(&self, folder: &FolderRow, cx: &Context<Self>) -> impl IntoElement {
        let library = self.library.read(cx);
        let inside = library
            .state(self.shelf())
            .outline()
            .playlists_in(&folder.id);
        let path = library.state(self.shelf()).outline().path(&folder.id);

        let mut strip = HeroMetaStrip::new();
        if path.len() > 1 {
            strip = strip.item(self.trail(&path[..path.len() - 1], cx));
        }
        strip = strip.text(cards::holding(folder));

        PageHero::new("folder-hero", folder.name.clone())
            .cover(library.folder_cover(&folder.id))
            .fallback(FOLDER)
            .eyebrow(t!("kind-folder"))
            .pin(Some(cards::folder_pin(
                folder,
                library.folder_cover(&folder.id),
            )))
            .meta(strip)
            .actions(self.shuffle_button(folder, inside, cx))
    }

    /// The folders this one sits in, each one a way back up.
    fn trail(&self, path: &[FolderRow], cx: &Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let mut trail = div().flex().items_center().gap_1();

        for (place, step) in path.iter().enumerate() {
            let opened = SharedString::from(step.id.clone());
            trail = trail.when(place > 0, |this| this.child("/")).child(
                div()
                    .id(("folder-trail", place))
                    .cursor_pointer()
                    .hover(move |style| style.text_color(theme.foreground))
                    .child(SharedString::from(step.name.clone()))
                    .on_click(move |_, _, cx| navigate(Destination::Folder(opened.clone()), cx)),
            );
        }

        trail
    }

    fn shuffle_button(
        &self,
        folder: &FolderRow,
        inside: Vec<usize>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let held = self.library.read(cx).state(self.shelf());
        let playlists: Vec<String> = inside
            .into_iter()
            .filter_map(|index| held.playlists().get(index))
            .map(|playlist| playlist.id.clone())
            .collect();

        let gathering = self.shuffle.read(cx).gathering();
        let shuffle = self.shuffle.clone();
        let shelf = self.shelf();
        let label = match gathering {
            true => t!("play-loading"),
            false => t!("play-shuffle"),
        };

        div().flex().child(
            Button::new(("folder-shuffle", folder.id.len()))
                .label(label)
                .icon("icons/shuffle.svg")
                .primary()
                .disabled(gathering || playlists.is_empty())
                .on_click(move |_, _, cx| {
                    let playlists = playlists.clone();
                    shuffle.update(cx, |shuffle, cx| shuffle.play(shelf, playlists, cx));
                }),
        )
    }

    fn cards(&self, cx: &Context<Self>) -> impl IntoElement {
        let (folders, playlists) = self.contents(cx);
        let layout = CardGrid::layout(self.width);
        let mut cards: Vec<_> = Vec::with_capacity(folders.len() + playlists.len());

        for (place, folder) in folders.iter().enumerate() {
            let cover = self.library.read(cx).folder_cover(&folder.id);
            let summoned = Summoned::Folder(folder.clone());
            let view = cx.entity().downgrade();
            cards.push(
                cards::folder_card(("folder-inside", place), folder, cover)
                    .tile(layout.card)
                    .flat()
                    .menu(move |event, _, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        let summoned = summoned.clone();
                        let position = event.position;
                        view.update(cx, |this, cx| {
                            this.context_menu = Some((summoned, position));
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            );
        }
        for (place, playlist) in playlists.iter().enumerate() {
            let summoned = Summoned::Playlist(playlist.clone());
            let view = cx.entity().downgrade();
            cards.push(
                cards::playlist_card(("folder-playlist", place), playlist, &self.playback, cx)
                    .tile(layout.card)
                    .flat()
                    .menu(move |event, _, cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        let summoned = summoned.clone();
                        let position = event.position;
                        view.update(cx, |this, cx| {
                            this.context_menu = Some((summoned, position));
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_wrap()
            .w_full()
            .gap_x(layout.gap)
            .gap_y_6()
            .children(cards)
    }
}

impl Render for FolderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let pad = theme.metrics.inset;
        let room = cells::content_width(window, pad * 2., cx);
        if (room - self.width).abs() >= STEADY {
            self.width = room;
        }

        let folder = self.folder(cx);
        let (folders, playlists) = self.contents(cx);
        let empty = folders.is_empty() && playlists.is_empty();
        let context_menu = self.context_menu.clone().map(|(summoned, position)| {
            let menu = match &summoned {
                Summoned::Playlist(playlist) => {
                    playlist_menu(playlist.clone(), self.playback.clone(), false, cx)
                }
                Summoned::Folder(folder) => folder_menu(&folder.id, &folder.name, cx),
            };
            Popup::new(position, menu).on_close(cx.listener(|this, _, _, cx| {
                this.context_menu = None;
                cx.notify();
            }))
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .when_some(context_menu, |this, menu| this.child(menu))
            .child(
                Scroller::new("folder-page", &self.scrollbar).p(pad).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_8()
                        .when_some(folder.as_ref(), |this, folder| {
                            this.child(self.header(folder, cx))
                        })
                        .when(folder.is_none(), |this| {
                            this.child(Vacancy::new(t!("folder-missing")).icon(FOLDER))
                        })
                        .when(folder.is_some() && empty, |this| {
                            this.child(Vacancy::new(t!("folder-empty")).icon(FOLDER))
                        })
                        .when(!empty, |this| this.child(self.cards(cx))),
                ),
            )
    }
}
