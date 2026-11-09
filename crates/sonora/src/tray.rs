//! The tray icon and its menu: the playing song with its cover, transport, shuffle and repeat,
//! and the way back to the window. The platform halves live in `sni` (Linux, StatusNotifierItem
//! over D-Bus) and `native` (macOS and Windows, through `tray-icon`); this module keeps them in
//! step with the store.
//!
//! Nothing here observes the store: [`publish`] runs from the app's `poll`, which Rune calls
//! before every frame and on every wake of a hidden window, and compares a hash of what the
//! menu shows before it builds anything. A menu event reaches the store through
//! `AppHandle::update`, like every other thread does.

#[cfg(any(target_os = "macos", windows))]
mod native;
#[cfg(target_os = "linux")]
mod sni;

use std::cell::RefCell;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use i18n::t;
use router::Destination;
use rune::Tx;
use state::{Outcome, PlaybackState, Repeat, Store, Toasts};
use tokio::task::JoinHandle;
use views::Ui;

#[cfg(any(target_os = "macos", windows))]
use native::Icon;
#[cfg(target_os = "linux")]
use sni::Icon;

const CAPTION_LIMIT: usize = 26;

/// The longest side of the cover handed to the menu. macOS draws it 18pt tall whatever it
/// measures, Windows and the status notifier hosts draw it at its own size.
const COVER: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Show,
    Song,
    Toggle,
    Previous,
    Next,
    Shuffle,
    Repeat,
    Quit,
}

/// The cover of the playing track, decoded and scaled for a menu row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Art {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Everything the menu draws, compared whole so a change that alters nothing visible costs no
/// platform call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub artwork: Option<Arc<Art>>,
    pub caption: String,
    /// Whether the caption opens anything, so a row that leads nowhere stays inert.
    pub song: bool,
    pub toggle: String,
    pub previous: String,
    pub next: String,
    pub shuffle: String,
    pub shuffle_on: bool,
    pub repeat: String,
    pub repeat_on: bool,
    pub show: String,
    pub quit: String,
    pub playing: bool,
}

impl Shown {
    /// What the Dock menu draws, so the native menus are rebuilt only when this changes, never
    /// on a caption or cover alone.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn docked(&self) -> (&str, &str, &str, &str, bool, &str, bool) {
        (
            &self.toggle,
            &self.previous,
            &self.next,
            &self.shuffle,
            self.shuffle_on,
            &self.repeat,
            self.repeat_on,
        )
    }
}

thread_local! {
    /// The tray, owned by the UI thread: every update, poll and window hook runs there.
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

struct Tray {
    icon: Icon,
    /// Whether the desktop has a tray to bring Sonora back from.
    hosted: bool,
    shown: Shown,
    /// The hash of what `shown` was built from, so a poll with nothing new builds nothing.
    seen: Option<u64>,
    /// Whether the icon is in the tray now, which `place` compares against the setting.
    placed: bool,
    /// The cover the art below was loaded from, so a repeat of the same track loads nothing.
    cover: Option<String>,
    art: Option<Arc<Art>>,
    /// The cover load in flight; aborting it on a skip means a run of skips draws only the last.
    artwork: Option<JoinHandle<()>>,
    http: crate::http::Client,
}

/// Starts the tray: the platform probe (a D-Bus round trip on Linux) runs off the UI thread,
/// then the icon is built on it, since AppKit and the Win32 tray want the main thread.
pub fn start(ui: &Ui, http: crate::http::Client) {
    let io = ui.store.cx.io.clone();
    io.spawn(async move {
        let hosted = tokio::task::spawn_blocking(Icon::probe).await.ok().flatten();
        crate::handle().update(move |ui: &mut Ui, _| {
            if let Some(hosted) = hosted {
                install(ui, hosted, http);
            }
        });
    });
}

/// Whether closing the window should leave Sonora running: only while the icon is in a tray
/// that can bring the window back.
pub fn lingers() -> bool {
    TRAY.with_borrow(|tray| tray.as_ref().is_some_and(|tray| tray.hosted && tray.placed))
}

/// Keeps the icon placed as the setting says and the menu in step with playback. Cheap when
/// nothing the menu shows changed, which is almost every frame.
pub fn publish(ui: &mut Ui) {
    let placed = ui.store.settings.tray_icon();
    let seen = fingerprint(&ui.store);
    let refused = TRAY.with_borrow_mut(|tray| {
        let tray = tray.as_mut()?;
        let refused = tray.place(placed);
        if tray.seen != Some(seen) {
            tray.seen = Some(seen);
            tray.follow(&ui.store);
            tray.refresh(&ui.store);
        }
        refused
    });
    // turning the setting off happens outside the borrow, since it settles the store
    if refused.is_some() {
        Toasts::show(Outcome::Failed, "toast-tray-unavailable", &mut ui.store.cx);
        ui.store.settings.set_tray_icon(false, &mut ui.store.cx);
        ui.settle();
    }
}

/// Carries out what the tray menu asked for, on the UI thread.
pub fn run(ui: &mut Ui, event: Event, tx: &mut Tx) {
    match event {
        Event::Show => crate::window::show(),
        Event::Song => {
            crate::window::show();
            open_song(ui);
        }
        Event::Quit => return crate::window::quit(ui, tx),
        Event::Toggle => ui.store.toggle_play(),
        Event::Previous => ui.store.previous_track(),
        Event::Next => ui.store.next_track(),
        Event::Shuffle => ui.store.toggle_shuffle(),
        Event::Repeat => ui.store.toggle_repeat(),
    }
    ui.settle();
    publish(ui);
}

/// Hands a menu event from the platform's thread to the UI thread.
pub(crate) fn send(event: Event) {
    crate::handle().update(move |ui: &mut Ui, tx| run(ui, event, tx));
}

fn install(ui: &mut Ui, hosted: bool, http: crate::http::Client) {
    let Some(mut icon) = Icon::new() else {
        return;
    };
    let shown = shown(None, &ui.store);
    icon.show(&shown);
    crate::actions::publish(&shown);
    let tray = Tray {
        icon,
        hosted,
        shown,
        seen: None,
        placed: false,
        cover: None,
        art: None,
        artwork: None,
        http,
    };
    TRAY.with_borrow_mut(|slot| *slot = Some(tray));
    publish(ui);
}

impl Tray {
    /// Puts the icon in the tray, or takes it out, following `tray_icon` alone. Returns the
    /// error when the icon was asked for and cannot be placed, which turns the setting off.
    fn place(&mut self, placed: bool) -> Option<anyhow::Error> {
        if placed == self.placed {
            return None;
        }
        if let Err(error) = self.icon.place(placed) {
            log::warn!("tray: cannot place the tray icon: {error:#}");
            if placed {
                // the setting goes off, which ends up here again with nothing to do
                return Some(error);
            }
        }
        self.placed = placed;
        if placed {
            // the cover of whatever is playing went unfetched while the icon was out, so
            // forget the one `follow` last saw and let it load again
            if self.art.is_none() {
                self.cover = None;
            }
            self.seen = None;
        }
        None
    }

    /// Rebuilds what the menu shows and hands it over when it changed.
    fn refresh(&mut self, store: &Store) {
        let shown = shown(self.art.clone(), store);
        if shown == self.shown {
            return;
        }
        self.icon.show(&shown);
        if shown.docked() != self.shown.docked() {
            crate::actions::publish(&shown);
        }
        self.shown = shown;
    }

    /// Starts loading the cover of the track that is playing now, dropping the load for the
    /// track that played before.
    fn follow(&mut self, store: &Store) {
        let cover = store
            .playback
            .shown_track(&store.devices)
            .and_then(|track| track.cover);
        if cover == self.cover {
            return;
        }

        self.cover = cover.clone();
        self.art = None;
        if let Some(load) = self.artwork.take() {
            load.abort();
        }
        // only the tray menu draws the cover, so an icon that is out of the tray fetches none
        let (true, Some(cover)) = (self.placed, cover) else {
            return;
        };
        let http = self.http.clone();
        self.artwork = Some(store.cx.io.spawn(async move {
            let art = match art(&cover, &http).await {
                Ok(art) => Arc::new(art),
                Err(error) => return log::warn!("tray: cannot draw the cover: {error:#}"),
            };
            crate::handle().update(move |ui: &mut Ui, _| drawn(ui, &cover, art));
        }));
    }
}

/// Puts a loaded cover on the caption row, if its track is still the one playing.
fn drawn(ui: &mut Ui, cover: &str, art: Arc<Art>) {
    TRAY.with_borrow_mut(|tray| {
        let Some(tray) = tray.as_mut() else {
            return;
        };
        if tray.cover.as_deref() != Some(cover) {
            return;
        }
        tray.art = Some(art);
        tray.artwork = None;
        tray.refresh(&ui.store);
    });
}

/// Opens the song page of whatever is playing. A track without an id is not an error: the
/// caption row is only enabled when there is one.
fn open_song(ui: &mut Ui) {
    let playing = ui.store.playback.shown_track(&ui.store.devices);
    let Some(id) = playing.and_then(|track| track.id) else {
        return;
    };
    ui.go(Destination::Song(id.as_str().into()));
}

/// Reads a cover, over http or from the disk, and scales it down to a menu row. An unreadable
/// cover leaves the caption on its own rather than holding up the rest of the menu.
async fn art(cover: &str, http: &crate::http::Client) -> Result<Art> {
    let bytes = match cover.starts_with("http://") || cover.starts_with("https://") {
        true => http
            .fetch(cover.to_owned())
            .await
            .context("cannot fetch the cover")?,
        false => {
            let path = cover.strip_prefix("file://").unwrap_or(cover).to_owned();
            tokio::fs::read(path)
                .await
                .context("cannot read the cover")?
        }
    };
    tokio::task::spawn_blocking(move || {
        let image = image::load_from_memory(&bytes)
            .context("cannot decode the cover")?
            .thumbnail(COVER, COVER)
            .into_rgba8();
        let (width, height) = image.dimensions();
        Ok(Art {
            data: image.into_raw(),
            width,
            height,
        })
    })
    .await
    .context("cannot scale the cover")?
}

/// Whether the track is playing, as the player shows it: the other device's state while one is
/// steered.
fn playing(store: &Store) -> bool {
    match store.playback.steered(&store.devices) {
        Some(remote) => remote.state.playing,
        None => matches!(
            store.playback.state(),
            PlaybackState::Playing | PlaybackState::Loading
        ),
    }
}

/// A hash of everything [`shown`] reads, which costs no allocation for this app's own track.
fn fingerprint(store: &Store) -> u64 {
    let mut hasher = DefaultHasher::new();
    playing(store).hash(&mut hasher);
    store.queue.shuffle().hash(&mut hasher);
    (store.playback.repeat() != Repeat::Off).hash(&mut hasher);
    store.settings.language().hash(&mut hasher);
    let hash = |track: &music::Track, hasher: &mut DefaultHasher| {
        (&track.id, &track.name, &track.artists, &track.cover).hash(hasher);
    };
    match store.playback.steered(&store.devices) {
        Some(_) => match store.playback.shown_track(&store.devices) {
            Some(track) => hash(&track, &mut hasher),
            None => 0u8.hash(&mut hasher),
        },
        None => match store.playback.track() {
            Some(track) => hash(track, &mut hasher),
            None => 0u8.hash(&mut hasher),
        },
    }
    hasher.finish()
}

fn shown(artwork: Option<Arc<Art>>, store: &Store) -> Shown {
    let playing = playing(store);
    let track = store.playback.shown_track(&store.devices);
    let caption = match &track {
        Some(track) => {
            let full = match track.artists.is_empty() {
                true => track.name.clone(),
                false => format!("{} – {}", track.artists, track.name),
            };
            clip(&full, CAPTION_LIMIT)
        }
        None => t!("player-nothing-playing").to_string(),
    };
    let song = track.is_some_and(|track| track.id.is_some());
    Shown {
        artwork,
        caption,
        song,
        toggle: match playing {
            true => t!("tray-pause"),
            false => t!("tray-play"),
        }
        .to_string(),
        previous: t!("player-previous").to_string(),
        next: t!("player-next").to_string(),
        shuffle: t!("player-shuffle").to_string(),
        shuffle_on: store.queue.shuffle(),
        repeat: t!("player-repeat").to_string(),
        repeat_on: store.playback.repeat() != Repeat::Off,
        show: t!("tray-show").to_string(),
        quit: t!("app-quit").to_string(),
        playing,
    }
}

/// Truncates `text` to at most `limit` characters, appending an ellipsis if shortened.
fn clip(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some(_) => {
            let cut = limit.saturating_sub(1);
            let offset = text
                .char_indices()
                .nth(cut)
                .map(|(i, _)| i)
                .unwrap_or(text.len());
            let trimmed = text[..offset].trim_end_matches(|c: char| {
                c.is_whitespace() || c == '-' || c == '–' || c == '—' || c == ',' || c == '('
            });
            format!("{trimmed}…")
        }
        None => text.to_string(),
    }
}
