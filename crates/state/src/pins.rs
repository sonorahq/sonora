use std::cell::OnceCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use music::{PinTarget, PinTargetKind};
use ui::{Pin, PinKind};

use crate::library::{Library, LibraryEvent, Shelf};
use crate::session::Session;
use crate::settings::AppSettings;
use crate::{Change, Cx, Event, Outcome, Store, Target, Toast, Toasts, Topic};

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
    /// What the provider last reported as pinned, by uri, so only a change since then is
    /// followed.
    mirrored: HashMap<String, Pin>,
    /// `entries` as last laid out. Cleared whenever the settings, the session or the library
    /// move, so a frame reads the list without rebuilding it.
    laid: OnceCell<Arc<Vec<Pin>>>,
    /// `library` as last laid out, cleared alongside `laid`.
    rest: OnceCell<Arc<Vec<Pin>>>,
    /// The digest of what the shelves last listed, so a library change that changed nothing
    /// the sidebar shows does not rebuild it.
    seen: u64,
    /// Pins that left with a playlist taken out of the library, by playlist id, so an Undo
    /// that brings the playlist back brings its pin back to the same slot.
    parted: HashMap<String, Parted>,
    /// The toast reporting the last unpin, and the pin it reports.
    unpinned: Option<(usize, Pin)>,
}

/// A pin as it was stored, with its provider and its slot in the dragged order.
#[derive(Clone)]
pub(crate) struct Parted {
    slug: &'static str,
    pin: Pin,
    gap: usize,
}

/// The models the pinned list reads and changes, borrowed apart from the store.
struct Parts<'a> {
    settings: &'a mut AppSettings,
    library: &'a mut Library,
    session: &'a Session,
    cx: &'a mut Cx,
}

impl Pins {
    pub fn new() -> Self {
        Self {
            mirrored: HashMap::new(),
            laid: OnceCell::new(),
            rest: OnceCell::new(),
            seen: 0,
            parted: HashMap::new(),
            unpinned: None,
        }
    }

    /// Every pin of the live providers, laid out the way the user asked for. The list is built
    /// once per change and shared afterwards.
    pub fn entries(&self, settings: &AppSettings, session: &Session) -> Arc<Vec<Pin>> {
        self.laid
            .get_or_init(|| {
                let slugs = session.active_slugs();
                let mut pinned = settings.pinned(&slugs);
                lay_out(&mut pinned, settings);
                Arc::new(pinned)
            })
            .clone()
    }

    /// Everything the live shelves hold that is not pinned: albums, artists and playlists, laid
    /// out the same way the pins above them are. Nothing else a provider lists belongs here,
    /// since the sidebar can only open these three. Built once per change, like `entries`.
    pub fn library(
        &self,
        settings: &AppSettings,
        session: &Session,
        library: &Library,
    ) -> Arc<Vec<Pin>> {
        self.rest
            .get_or_init(|| Arc::new(gather_library(settings, session, library)))
            .clone()
    }

    /// Drops both laid-out lists so the next read rebuilds them.
    fn forget(&mut self) {
        self.laid.take();
        self.rest.take();
    }

    /// Forgets the laid-out lists and tells the followers.
    fn changed(&mut self, cx: &mut Cx) {
        self.forget();
        cx.changed(Topic::Pins);
    }

    /// The automatic order in effect, or `None` for the order the user dragged.
    pub fn sort(&self, settings: &AppSettings) -> Option<PinSort> {
        settings.sidebar_pin_sort()
    }

    pub fn reversed(&self, settings: &AppSettings) -> bool {
        settings.sidebar_pin_reversed()
    }

    pub fn sorted(&self, settings: &AppSettings) -> bool {
        self.sort(settings).is_some()
    }

    pub fn holds(&self, pin: &Pin, settings: &AppSettings, session: &Session) -> bool {
        dragged(settings, session).iter().any(|held| held.same(pin))
    }

    /// Steps one order along: off, then down, then off again. Picking another order starts it
    /// from the top.
    fn choose(&mut self, sort: PinSort, parts: &mut Parts) {
        let next = match (
            self.sort(parts.settings) == Some(sort),
            self.reversed(parts.settings),
        ) {
            (true, false) => Some((sort, true)),
            (true, true) => None,
            (false, _) => Some((sort, false)),
        };
        let (sort, reversed) = match next {
            Some((sort, reversed)) => (Some(sort), reversed),
            None => (None, false),
        };
        parts
            .settings
            .set_sidebar_pin_sort(sort, reversed, parts.cx);
        self.changed(parts.cx);
    }

    fn toggle(&mut self, pin: Pin, parts: &mut Parts) {
        match self.holds(&pin, parts.settings, parts.session) {
            true => self.unpin(pin, parts),
            false => self.place(pin, None, parts),
        }
    }

    /// Pins at `gap`, a slot counted among the pins already shown, or moves an existing pin there.
    /// `None` appends, and never moves a pin that is already in the list.
    fn place(&mut self, pin: Pin, gap: Option<usize>, parts: &mut Parts) {
        let Some(slug) = parts.session.slug_for(&pin.id) else {
            return;
        };
        let known = self.holds(&pin, parts.settings, parts.session);
        match gap {
            Some(_) => self.freeze(parts),
            None if known => return,
            None => {}
        }
        let slugs = parts.session.active_slugs();
        parts.settings.pin(slug, pin.clone(), gap, &slugs, parts.cx);
        if !known {
            self.tell(&pin, true, parts);
        }
        self.changed(parts.cx);
    }

    /// Unpins and reports it in a toast whose Undo pins it back into the same slot.
    fn unpin(&mut self, pin: Pin, parts: &mut Parts) {
        let Some(slug) = parts.session.slug_for(&pin.id) else {
            return;
        };
        let parted = part(slug, &pin, parts.settings, parts.session);
        parts.settings.unpin(slug, &pin, parts.cx);
        self.tell(&pin, false, parts);
        self.changed(parts.cx);

        let Some(parted) = parted else {
            return;
        };
        let back = parted.clone();
        let toast = Toast::new(Outcome::Done, "toast-unpinned")
            .name(parted.pin.title.clone())
            .target(target(&parted.pin))
            .undo(move |store| store.restore_pin(back.clone()));
        self.unpinned = Some((Toasts::post(toast, parts.cx), parted.pin));
    }

    /// Pins a parted pin back into its slot, as an Undo does. Nothing happens when it is pinned
    /// already or its provider is no longer the live one.
    fn restore(&mut self, parted: Parted, parts: &mut Parts) {
        let Parted { slug, pin, gap } = parted;
        if self.holds(&pin, parts.settings, parts.session) {
            return;
        }
        let slugs = parts.session.active_slugs();
        if parts.session.slug_for(&pin.id) != Some(slug) || !slugs.contains(&slug) {
            return;
        }
        parts
            .settings
            .pin(slug, pin.clone(), Some(gap), &slugs, parts.cx);
        self.tell(&pin, true, parts);
        self.changed(parts.cx);
    }

    /// Mirrors a change into the provider's own pins. Nothing happens for a provider that keeps
    /// none, or for an item it does not carry in its library.
    fn tell(&mut self, pin: &Pin, pinned: bool, parts: &mut Parts) {
        let Some(uri) = remote(pin, parts.library, parts.session) else {
            return;
        };
        let pin = pin.clone();
        parts.library.set_pinned(
            uri,
            pinned,
            move |kept, store| {
                if kept {
                    return;
                }
                store.revert_pin(pin, pinned);
            },
            parts.session,
            parts.cx,
        );
    }

    /// Freezes the list the way it reads on screen and goes back to the dragged order, so a slot
    /// the user aimed at means the same thing in the store as it did on screen.
    fn freeze(&mut self, parts: &mut Parts) {
        if !self.sorted(parts.settings) {
            return;
        }
        let shown = self.entries(parts.settings, parts.session);
        let slugs = parts.session.active_slugs();
        parts.settings.rearrange(&shown, &slugs, parts.cx);
        parts.settings.set_sidebar_pin_sort(None, false, parts.cx);
        self.forget();
    }

    /// Puts the local list back after the provider refused the change, and takes down the
    /// toast that reported an unpin the provider turned away.
    fn revert(&mut self, pin: Pin, attempted: bool, parts: &mut Parts) {
        let Some(slug) = parts.session.slug_for(&pin.id) else {
            return;
        };
        if !attempted && let Some((toast, unpinned)) = self.unpinned.take() {
            match unpinned.same(&pin) {
                true => Toasts::close(toast, parts.cx),
                false => self.unpinned = Some((toast, unpinned)),
            }
        }
        let slugs = parts.session.active_slugs();
        match attempted {
            true => parts.settings.unpin(slug, &pin, parts.cx),
            false => parts.settings.pin(slug, pin, None, &slugs, parts.cx),
        }
        self.changed(parts.cx);
    }

    /// Follows the provider's own pins: one it pinned elsewhere joins the local list, one it
    /// dropped elsewhere leaves it. Only a change since the last look counts, so a local pin the
    /// provider never held stays put, and the first look after signing in imports what is there.
    /// A dropped pin is found in `mirrored`, since a provider may stop listing it altogether.
    fn absorb(&mut self, parts: &mut Parts) {
        if parts.library.pin_pending() {
            return;
        }
        let Some(items) = parts.library.pin_targets() else {
            self.mirrored.clear();
            return;
        };
        let now: Vec<(String, Pin)> = items
            .iter()
            .filter(|item| item.pinned)
            .filter_map(|item| Some((item.uri.clone(), pin_of(item)?)))
            .collect();
        if now.len() == self.mirrored.len()
            && now.iter().all(|(uri, _)| self.mirrored.contains_key(uri))
        {
            return;
        }
        let session = parts.session;
        let slugs = session.active_slugs();
        let held = parts.settings.pinned(&slugs);
        let known = |pin: &Pin| held.iter().any(|known| known.same(pin));
        let arrived: Vec<_> = now
            .iter()
            .filter(|(uri, pin)| !self.mirrored.contains_key(uri) && !known(pin))
            .filter_map(|(uri, pin)| Some((session.slug_for(uri)?, pin.clone())))
            .collect();
        let left: Vec<_> = self
            .mirrored
            .iter()
            .filter(|(uri, pin)| !now.iter().any(|(held, _)| held == *uri) && known(pin))
            .filter_map(|(uri, pin)| Some((session.slug_for(uri)?, pin.clone())))
            .collect();
        self.mirrored = now.into_iter().collect();
        if arrived.is_empty() && left.is_empty() {
            return;
        }
        for (slug, pin) in left {
            parts.settings.unpin(slug, &pin, parts.cx);
        }
        for (slug, pin) in arrived {
            parts.settings.pin(slug, pin, None, &slugs, parts.cx);
        }
        self.changed(parts.cx);
    }
}

impl Default for Pins {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// Steps the pinned list's order along: off, then down, then off again. Picking another
    /// order starts it from the top.
    pub fn choose_pin_sort(&mut self, sort: PinSort) {
        let (pins, mut parts) = self.pin_parts();
        pins.choose(sort, &mut parts);
    }

    /// Pins `pin`, or unpins it when it is pinned already.
    pub fn toggle_pin(&mut self, pin: Pin) {
        let (pins, mut parts) = self.pin_parts();
        pins.toggle(pin, &mut parts);
    }

    /// Pins at `gap`, a slot counted among the pins already shown, or moves an existing pin there.
    /// `None` appends, and never moves a pin that is already in the list.
    pub fn place_pin(&mut self, pin: Pin, gap: Option<usize>) {
        let (pins, mut parts) = self.pin_parts();
        pins.place(pin, gap, &mut parts);
    }

    /// Unpins and reports it in a toast whose Undo pins it back into the same slot.
    pub fn unpin(&mut self, pin: Pin) {
        let (pins, mut parts) = self.pin_parts();
        pins.unpin(pin, &mut parts);
    }

    /// Pins a parted pin back into its slot, as the unpin toast's Undo does.
    fn restore_pin(&mut self, parted: Parted) {
        let (pins, mut parts) = self.pin_parts();
        pins.restore(parted, &mut parts);
    }

    /// Puts the local list back after the provider refused a pin change.
    fn revert_pin(&mut self, pin: Pin, attempted: bool) {
        let (pins, mut parts) = self.pin_parts();
        pins.revert(pin, attempted, &mut parts);
    }

    /// The pinned list and the models it reads and changes, borrowed apart.
    fn pin_parts(&mut self) -> (&mut Pins, Parts<'_>) {
        let Store {
            pins,
            settings,
            library,
            session,
            cx,
            ..
        } = self;
        let parts = Parts {
            settings,
            library,
            session,
            cx,
        };
        (pins, parts)
    }

    pub(crate) fn react_pins(&mut self, change: &Change) {
        let (pins, mut parts) = self.pin_parts();
        match change {
            Change::Event(Event::Library(LibraryEvent::PlaylistGone(id))) => {
                let pin = Pin::new(PinKind::Playlist, id, "");
                if let Some(slug) = parts.session.slug_for(id) {
                    if let Some(parted) = part(slug, &pin, parts.settings, parts.session) {
                        pins.parted.insert(id.clone(), parted);
                    }
                    parts.settings.unpin(slug, &pin, parts.cx);
                }
                pins.mirrored.retain(|_, held| !held.same(&pin));
                pins.changed(parts.cx);
            }
            Change::Event(Event::Library(LibraryEvent::PlaylistBack(id))) => {
                if let Some(parted) = pins.parted.remove(id) {
                    pins.restore(parted, &mut parts);
                }
            }
            Change::Topic(Topic::Library) => {
                pins.absorb(&mut parts);
                let seen = fingerprint(parts.library, parts.session);
                if pins.seen != seen {
                    pins.seen = seen;
                    pins.changed(parts.cx);
                }
            }
            Change::Topic(Topic::Settings) | Change::Topic(Topic::Session) => {
                pins.changed(parts.cx)
            }
            _ => {}
        }
    }
}

/// A digest of the albums, artists and playlists the live shelves list, with the names
/// and covers the sidebar shows for them.
fn fingerprint(library: &Library, session: &Session) -> u64 {
    let mut hasher = DefaultHasher::new();
    for shelf in [Shelf::Streaming, Shelf::Local] {
        if session.client_of(shelf).is_none() {
            continue;
        }
        let held = library.state(shelf);
        for playlist in held.playlists() {
            (&playlist.id, &playlist.name, &playlist.cover).hash(&mut hasher);
        }
        for album in held.albums() {
            (&album.id, &album.name, &album.cover_large, &album.cover).hash(&mut hasher);
        }
        for artist in held.artists() {
            (&artist.id, &artist.name, &artist.cover).hash(&mut hasher);
        }
    }
    hasher.finish()
}

fn gather_library(settings: &AppSettings, session: &Session, library: &Library) -> Vec<Pin> {
    let pinned = dragged(settings, session);
    let mut rest = Vec::new();
    for shelf in [Shelf::Streaming, Shelf::Local] {
        if session.client_of(shelf).is_none() {
            continue;
        }
        let held = library.state(shelf);
        rest.extend(held.playlists().iter().map(|playlist| {
            Pin::new(PinKind::Playlist, &playlist.id, &playlist.name).cover(playlist.cover.clone())
        }));
        rest.extend(held.albums().iter().map(|album| {
            Pin::new(PinKind::Album, &album.id, &album.name)
                .cover(album.cover_large.clone().or_else(|| album.cover.clone()))
        }));
        rest.extend(held.artists().iter().map(|artist| {
            Pin::new(PinKind::Artist, &artist.id, &artist.name).cover(artist.cover.clone())
        }));
    }
    rest.retain(|pin| !pinned.iter().any(|held| held.same(pin)));
    lay_out(&mut rest, settings);
    rest
}

/// The pins in the order they are stored in, whatever the current sort shows.
fn dragged(settings: &AppSettings, session: &Session) -> Vec<Pin> {
    let slugs = session.active_slugs();
    settings.pinned(&slugs)
}

/// Applies the automatic order, if there is one. Without one the list keeps the order it
/// arrived in, which for the pins is the one the user dragged.
fn lay_out(pins: &mut [Pin], settings: &AppSettings) {
    match settings.sidebar_pin_sort() {
        Some(PinSort::Alphabetical) => pins.sort_by_key(|pin| pin.title.to_lowercase()),
        Some(PinSort::Kind) => pins.sort_by_key(|pin| (rank(pin.kind), pin.title.to_lowercase())),
        None => return,
    }
    if settings.sidebar_pin_reversed() {
        pins.reverse();
    }
}

/// The stored pin matching `pin` with the slot it holds, or `None` when it is not pinned.
fn part(
    slug: &'static str,
    pin: &Pin,
    settings: &AppSettings,
    session: &Session,
) -> Option<Parted> {
    let dragged = dragged(settings, session);
    let gap = dragged.iter().position(|held| held.same(pin))?;
    Some(Parted {
        slug,
        pin: dragged[gap].clone(),
        gap,
    })
}

/// The provider's own uri for a pin, when the provider keeps pins itself. A listed item
/// keeps the uri it was listed with, and the provider builds one for anything else. A
/// provider that keeps no pins, as YouTube does, answers `None`, so the pin stays a local
/// one rather than failing on a call it cannot make.
fn remote(pin: &Pin, library: &Library, session: &Session) -> Option<String> {
    if !session.capabilities().pins || music::is_local_id(&pin.id) {
        return None;
    }
    let listed = library
        .pin_targets()?
        .iter()
        .find(|item| pin_of(item).is_some_and(|listed| listed.same(pin)))
        .map(|item| item.uri.clone());
    listed.or_else(|| {
        session
            .client_of(Shelf::Streaming)?
            .pin_uri(target_kind(pin.kind)?, &pin.id)
    })
}

/// The page a pin's toast links its name to.
fn target(pin: &Pin) -> Option<Target> {
    let id = pin.id.clone().into();
    Some(match pin.kind {
        PinKind::Playlist => Target::Playlist(id),
        PinKind::Album => Target::Album(id),
        PinKind::Artist => Target::Artist(id),
        PinKind::Song => Target::Song(id),
    })
}

fn rank(kind: PinKind) -> u8 {
    match kind {
        PinKind::Playlist => 0,
        PinKind::Album => 1,
        PinKind::Artist => 2,
        PinKind::Song => 3,
    }
}

/// The pin a provider's pin target stands for, or `None` for a row Sonora cannot open on its own,
/// such as a folder or a podcast.
fn pin_of(item: &PinTarget) -> Option<Pin> {
    let kind = match item.kind {
        PinTargetKind::Playlist => PinKind::Playlist,
        PinTargetKind::Album => PinKind::Album,
        PinTargetKind::Artist => PinKind::Artist,
        _ => return None,
    };
    let (_, id) = item.uri.rsplit_once(':')?;

    Some(Pin::new(kind, id, item.name.clone()).cover(item.cover.clone()))
}

/// The provider's kind for a pin, or `None` for a song, which no provider pins.
fn target_kind(kind: PinKind) -> Option<PinTargetKind> {
    match kind {
        PinKind::Playlist => Some(PinTargetKind::Playlist),
        PinKind::Album => Some(PinTargetKind::Album),
        PinKind::Artist => Some(PinTargetKind::Artist),
        PinKind::Song => None,
    }
}

#[cfg(test)]
mod tests {
    use super::pin_of;
    use music::{PinTarget, PinTargetKind};
    use ui::PinKind;

    fn item(uri: &str, kind: PinTargetKind) -> PinTarget {
        PinTarget {
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
        let pin = pin_of(&item("spotify:album:4aB", PinTargetKind::Album)).unwrap();
        assert_eq!(pin.kind, PinKind::Album);
        assert_eq!(pin.id, "4aB");
        assert!(pin_of(&item("spotify:show:4aB", PinTargetKind::Show)).is_none());
        assert!(pin_of(&item("bare", PinTargetKind::Album)).is_none());
    }
}
