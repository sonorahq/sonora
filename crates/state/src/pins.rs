use std::collections::HashSet;

use gpui::{App, Context, Entity};
use music::{LibraryItem, LibraryItemKind, Playlist};
use ui::{Pin, PinKind};

use crate::library::{Library, Shelf};
use crate::outline::{Outline, PlaylistRow};
use crate::session::Session;
use crate::settings::AppSettings;

/// An automatic order for the pinned list. No sort at all means the order the user dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinSort {
    Alphabetical,
    Kind,
}

impl PinSort {
    pub const ALL: [Self; 2] = [Self::Alphabetical, Self::Kind];

    /// The stored id. Anything else, the empty string included, means the dragged order.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Alphabetical => "alphabetical",
            Self::Kind => "kind",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Alphabetical => "nav-pins-alphabetical",
            Self::Kind => "nav-pins-kind",
        }
    }
}

/// The one pinned list, whatever provider each entry came from. Order is kept locally and covers
/// every entry at once, so a provider that cannot reorder its own pins still follows the drag, and
/// a streamed pin can sit between two local ones.
///
/// A provider that keeps pins of its own is told about every change, and the local list is put
/// back when it refuses.
pub struct Pins {
    settings: Entity<AppSettings>,
    library: Entity<Library>,
    session: Entity<Session>,
    /// The uris the provider last reported as pinned, so only a change since then is followed.
    mirrored: HashSet<String>,
}

impl Pins {
    pub fn new(
        settings: Entity<AppSettings>,
        library: Entity<Library>,
        session: Entity<Session>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&library, |this, _, cx| this.absorb(cx)).detach();
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();
        cx.observe(&session, |_, _, cx| cx.notify()).detach();

        Self {
            settings,
            library,
            session,
            mirrored: HashSet::new(),
        }
    }

    /// Every pin of the live providers, laid out the way the user asked for.
    pub fn entries(&self, cx: &App) -> Vec<Pin> {
        let slugs = self.session.read(cx).active_slugs();
        let mut pinned = self.settings.read(cx).pinned(&slugs);
        self.lay_out(&mut pinned, cx);
        pinned
    }

    /// Everything the live shelves hold that is not pinned: albums, artists, folders and the
    /// playlists that are not inside one, laid out the same way the pins above them are. Nothing
    /// else a provider lists belongs here, since the sidebar can only open these.
    ///
    /// A playlist inside a folder is left out on purpose: it is listed on that folder's page, and
    /// showing it here as well would leave the sidebar exactly as long as it was without folders.
    pub fn library(&self, cx: &App) -> Vec<Pin> {
        let pinned = self.dragged(cx);
        let library = self.library.read(cx);
        let mut rest = Vec::new();
        for shelf in [Shelf::Streaming, Shelf::Local] {
            if self.session.read(cx).client_of(shelf).is_none() {
                continue;
            }
            let held = library.state(shelf);
            rest.extend(top_level(held.outline(), held.playlists()));
            rest.extend(held.albums().iter().map(|album| {
                Pin::new(PinKind::Album, &album.id, &album.name)
                    .cover(album.cover_large.clone().or_else(|| album.cover.clone()))
            }));
            rest.extend(held.artists().iter().map(|artist| {
                Pin::new(PinKind::Artist, &artist.id, &artist.name).cover(artist.cover.clone())
            }));
        }
        rest.retain(|pin| !pinned.iter().any(|held| held.same(pin)));
        self.lay_out(&mut rest, cx);
        rest
    }

    /// The pins in the order they are stored in, whatever the current sort shows.
    fn dragged(&self, cx: &App) -> Vec<Pin> {
        let slugs = self.session.read(cx).active_slugs();
        self.settings.read(cx).pinned(&slugs)
    }

    /// The automatic order in effect, or `None` for the order the user dragged.
    pub fn sort(&self, cx: &App) -> Option<PinSort> {
        self.settings.read(cx).sidebar_pin_sort()
    }

    pub fn reversed(&self, cx: &App) -> bool {
        self.settings.read(cx).sidebar_pin_reversed()
    }

    pub fn sorted(&self, cx: &App) -> bool {
        self.sort(cx).is_some()
    }

    /// Applies the automatic order, if there is one. Without one the list keeps the order it
    /// arrived in, which for the pins is the one the user dragged.
    fn lay_out(&self, pins: &mut [Pin], cx: &App) {
        match self.sort(cx) {
            Some(PinSort::Alphabetical) => pins.sort_by_key(|pin| pin.title.to_lowercase()),
            Some(PinSort::Kind) => {
                pins.sort_by_key(|pin| (rank(pin.kind), pin.title.to_lowercase()))
            }
            None => return,
        }
        if self.reversed(cx) {
            pins.reverse();
        }
    }

    /// Steps one order along: off, then down, then off again. Picking another order starts it
    /// from the top.
    pub fn choose(&mut self, sort: PinSort, cx: &mut Context<Self>) {
        let next = match (self.sort(cx) == Some(sort), self.reversed(cx)) {
            (true, false) => Some((sort, true)),
            (true, true) => None,
            (false, _) => Some((sort, false)),
        };
        let (sort, reversed) = match next {
            Some((sort, reversed)) => (Some(sort), reversed),
            None => (None, false),
        };
        self.settings.update(cx, |settings, cx| {
            settings.set_sidebar_pin_sort(sort, reversed, cx)
        });
        cx.notify();
    }

    /// Whether a shelf knows the pin. Only folders are checked: the provider's own library names
    /// them apart from the playlist list Sonora reads, so one it offers may have no page here.
    fn listed(&self, pin: &Pin, cx: &App) -> bool {
        if pin.kind != PinKind::Folder {
            return true;
        }
        let library = self.library.read(cx);
        [Shelf::Streaming, Shelf::Local]
            .into_iter()
            .any(|shelf| library.state(shelf).outline().folder(&pin.id).is_some())
    }

    pub fn holds(&self, pin: &Pin, cx: &App) -> bool {
        self.dragged(cx).iter().any(|held| held.same(pin))
    }

    pub fn toggle(&mut self, pin: Pin, cx: &mut Context<Self>) {
        match self.holds(&pin, cx) {
            true => self.unpin(pin, cx),
            false => self.place(pin, None, cx),
        }
    }

    /// Pins at `gap`, a slot counted among the pins already shown, or moves an existing pin there.
    /// `None` appends, and never moves a pin that is already in the list.
    pub fn place(&mut self, pin: Pin, gap: Option<usize>, cx: &mut Context<Self>) {
        let Some(slug) = self.session.read(cx).slug_for(&pin.id) else {
            return;
        };
        let known = self.holds(&pin, cx);
        match gap {
            Some(_) => self.settle(cx),
            None if known => return,
            None => {}
        }
        let slugs = self.session.read(cx).active_slugs();
        self.settings.update(cx, |settings, cx| {
            settings.pin(slug, pin.clone(), gap, &slugs, cx)
        });
        if !known {
            self.tell(&pin, true, cx);
        }
        cx.notify();
    }

    pub fn unpin(&mut self, pin: Pin, cx: &mut Context<Self>) {
        let Some(slug) = self.session.read(cx).slug_for(&pin.id) else {
            return;
        };
        self.settings
            .update(cx, |settings, cx| settings.unpin(slug, &pin, cx));
        self.tell(&pin, false, cx);
        cx.notify();
    }

    /// Mirrors a change into the provider's own pins. Nothing happens for a provider that keeps
    /// none, or for an item it does not carry in its library.
    fn tell(&mut self, pin: &Pin, pinned: bool, cx: &mut Context<Self>) {
        let Some(uri) = self.remote(pin, cx) else {
            return;
        };
        let mine = cx.entity();
        let pin = pin.clone();
        self.library.update(cx, |library, cx| {
            library.set_sidebar_pinned(
                uri,
                pinned,
                move |kept, cx| {
                    if kept {
                        return;
                    }
                    mine.update(cx, |this, cx| this.revert(pin, pinned, cx));
                },
                cx,
            );
        });
    }

    /// Freezes the list the way it reads on screen and goes back to the dragged order, so a slot
    /// the user aimed at means the same thing in the store as it did on screen.
    fn settle(&mut self, cx: &mut Context<Self>) {
        if !self.sorted(cx) {
            return;
        }
        let shown = self.entries(cx);
        let slugs = self.session.read(cx).active_slugs();
        self.settings.update(cx, |settings, cx| {
            settings.rearrange(&shown, &slugs, cx);
            settings.set_sidebar_pin_sort(None, false, cx);
        });
    }

    /// Puts the local list back after the provider refused the change.
    fn revert(&mut self, pin: Pin, attempted: bool, cx: &mut Context<Self>) {
        let Some(slug) = self.session.read(cx).slug_for(&pin.id) else {
            return;
        };
        let slugs = self.session.read(cx).active_slugs();
        self.settings.update(cx, |settings, cx| match attempted {
            true => settings.unpin(slug, &pin, cx),
            false => settings.pin(slug, pin, None, &slugs, cx),
        });
        cx.notify();
    }

    /// The provider's own uri for a pin, when the provider lists it and keeps pins itself.
    fn remote(&self, pin: &Pin, cx: &App) -> Option<String> {
        self.library
            .read(cx)
            .sidebar_items()?
            .iter()
            .find(|item| pin_of(item).is_some_and(|listed| listed.same(pin)))
            .map(|item| item.uri.clone())
    }

    /// Follows the provider's own pins: one it pinned elsewhere joins the local list, one it
    /// dropped elsewhere leaves it. Only a change since the last look counts, so a local pin the
    /// provider never held stays put, and the first look after signing in imports what is there.
    fn absorb(&mut self, cx: &mut Context<Self>) {
        if self.library.read(cx).sidebar_pin_pending() {
            return;
        }
        let Some(items) = self
            .library
            .read(cx)
            .sidebar_items()
            .map(<[LibraryItem]>::to_vec)
        else {
            self.mirrored.clear();
            return;
        };
        let now: HashSet<String> = items
            .iter()
            .filter(|item| item.pinned)
            .map(|item| item.uri.clone())
            .collect();
        if now == self.mirrored {
            return;
        }
        let slugs = self.session.read(cx).active_slugs();
        let held = self.settings.read(cx).pinned(&slugs);
        let mut arrived = Vec::new();
        let mut left = Vec::new();
        for item in &items {
            let Some(pin) = pin_of(item).filter(|pin| self.listed(pin, cx)) else {
                continue;
            };
            let Some(slug) = self.session.read(cx).slug_for(&item.uri) else {
                continue;
            };
            let known = held.iter().any(|known| known.same(&pin));
            match (now.contains(&item.uri), self.mirrored.contains(&item.uri)) {
                (true, false) if !known => arrived.push((slug, pin)),
                (false, true) if known => left.push((slug, pin)),
                _ => {}
            }
        }
        self.mirrored = now;
        if arrived.is_empty() && left.is_empty() {
            return;
        }
        self.settings.update(cx, |settings, cx| {
            for (slug, pin) in left {
                settings.unpin(slug, &pin, cx);
            }
            for (slug, pin) in arrived {
                settings.pin(slug, pin, None, &slugs, cx);
            }
        });
        cx.notify();
    }
}

/// The folders and loose playlists of one shelf, in the order the provider listed them. A shelf
/// whose outline has not arrived yet lists every playlist, so nothing disappears while it loads.
fn top_level(outline: &Outline, playlists: &[Playlist]) -> Vec<Pin> {
    let playlist_pin = |playlist: &Playlist| {
        Pin::new(PinKind::Playlist, &playlist.id, &playlist.name).cover(playlist.cover.clone())
    };
    if outline.rows().is_empty() {
        return playlists.iter().map(playlist_pin).collect();
    }

    outline
        .level(None)
        .iter()
        .filter_map(|&at| match outline.row(at)? {
            PlaylistRow::Folder { id, name, .. } => Some(Pin::new(PinKind::Folder, id, name)),
            PlaylistRow::Playlist { index, .. } => playlists.get(*index).map(playlist_pin),
        })
        .collect()
}

fn rank(kind: PinKind) -> u8 {
    match kind {
        PinKind::Folder => 0,
        PinKind::Playlist => 1,
        PinKind::Album => 2,
        PinKind::Artist => 3,
        PinKind::Song => 4,
    }
}

/// The pin a provider's library row stands for, or `None` for a row Sonora cannot open on its own,
/// such as a podcast.
fn pin_of(item: &LibraryItem) -> Option<Pin> {
    let kind = match item.kind {
        LibraryItemKind::Playlist => PinKind::Playlist,
        LibraryItemKind::Album => PinKind::Album,
        LibraryItemKind::Artist => PinKind::Artist,
        LibraryItemKind::Folder => PinKind::Folder,
        _ => return None,
    };
    let (_, id) = item.uri.rsplit_once(':')?;

    Some(Pin::new(kind, id, item.name.clone()).cover(item.cover.clone()))
}

#[cfg(test)]
mod tests {
    use super::pin_of;
    use music::{LibraryItem, LibraryItemKind};
    use ui::PinKind;

    fn item(uri: &str, kind: LibraryItemKind) -> LibraryItem {
        LibraryItem {
            uri: uri.to_owned(),
            name: "name".into(),
            subtitle: "subtitle".into(),
            cover: None,
            kind,
            pinned: false,
        }
    }

    #[test]
    fn a_library_row_keeps_only_its_bare_id() {
        let pin = pin_of(&item("spotify:album:4aB", LibraryItemKind::Album)).unwrap();
        assert_eq!(pin.kind, PinKind::Album);
        assert_eq!(pin.id, "4aB");
        assert!(pin_of(&item("spotify:show:4aB", LibraryItemKind::Show)).is_none());
        assert!(pin_of(&item("bare", LibraryItemKind::Album)).is_none());
    }
}
