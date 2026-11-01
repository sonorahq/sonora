//! The previews of the pages where seeing a setting beats reading it. Each page with one has a
//! file here holding the preview's own state and the function that draws it, framed by `framed`.
//! The tab lays it across its column, under the page's title and above its first group.

use std::cell::{Cell, RefCell};
use std::hash::{BuildHasher as _, RandomState};
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{App, Context, Div, EdgesRefinement, Entity, Pixels, SharedString, Task, Window, div};
use i18n::t;
use state::Sonora;
use ui::ActiveTheme as _;

use super::SettingsView;
use crate::shared::ambient::Ambient;

pub(super) mod discord;
pub(super) mod lyrics;
pub(super) mod now_playing;
pub(super) mod sound;

/// How long the placeholder track runs.
const PLACEHOLDER_LENGTH: Duration = Duration::from_secs(214);
/// How far past a frame its corner ring reaches, as a share of the radius. It has to pass
/// the square root of two less one for the ring to cover the corners.
const CORNER_REACH: f32 = 0.5;

/// The state every preview keeps between frames, one field per preview, each owned by its file,
/// and what they share.
pub(super) struct Previews {
    pub(super) now_playing: now_playing::NowPlayingPreview,
    pub(super) lyrics: lyrics::LyricsPreview,
    pub(super) sound: sound::SoundPreview,
    pub(super) discord: discord::DiscordPreview,
    /// The ambient field the Now playing and Lyrics previews paint behind themselves, following
    /// the cover of the track they show.
    ambient: Entity<Ambient>,
    /// A random number drawn each time settings open, which picks the track from history.
    seed: Cell<u64>,
    /// The track from history the previews show while nothing plays, kept from the moment it
    /// is picked until settings open again.
    picked: RefCell<Option<Sample>>,
}

/// The track a preview shows, as much of it as the previews draw.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Sample {
    pub(super) id: Option<String>,
    pub(super) title: SharedString,
    pub(super) artist: SharedString,
    pub(super) album: SharedString,
    pub(super) album_id: Option<String>,
    pub(super) cover: Option<String>,
    pub(super) duration: Duration,
}

/// A redraw asked for after a wait, at most one at a time, for a preview that moves slower than
/// the display refreshes.
#[derive(Default)]
pub(super) struct Tick {
    pending: Rc<Cell<bool>>,
    task: RefCell<Option<Task<()>>>,
}

impl Previews {
    pub(super) fn new(cx: &mut Context<SettingsView>) -> Self {
        // history may still be loading when settings first draw, and a pick waits for it
        let history = Sonora::global(cx).history.clone();
        cx.observe(&history, |this, _, cx| {
            if this.previews.picked.borrow().is_none() {
                cx.notify();
            }
        })
        .detach();

        // the previews over the field take their colours from the cover it samples, so they
        // redraw once that lands
        let ambient = cx.new(Ambient::framed);
        cx.observe(&ambient, |_, _, cx| cx.notify()).detach();

        Self {
            now_playing: now_playing::NowPlayingPreview::new(cx),
            lyrics: lyrics::LyricsPreview::new(cx),
            sound: sound::SoundPreview::new(cx),
            discord: discord::DiscordPreview::new(cx),
            ambient,
            seed: Cell::new(random()),
            picked: RefCell::new(None),
        }
    }

    /// Settings opened: the next track from history is a new pick.
    pub(super) fn opened(&self) {
        self.seed.set(random());
        self.picked.replace(None);
        self.discord.opened();
    }

    /// Settings closed: whatever a preview keeps running only for itself stops.
    pub(super) fn closed(&mut self) {
        self.sound.hide();
    }

    /// The tab on show changed, so the previews of the others stop what they keep running.
    pub(super) fn shown(&mut self, tab: router::SettingsTab) {
        if tab != router::SettingsTab::Playback {
            self.sound.hide();
        }
    }

    /// The track playing, or while nothing is loaded the one picked from history this time
    /// settings opened, or a placeholder while history is empty.
    pub(super) fn playing_or_picked(&self, cx: &App) -> Sample {
        match Sonora::global(cx).playback.read(cx).track() {
            Some(track) => Sample::of(track),
            None => self.picked(cx),
        }
    }

    /// The track picked from history this time settings opened, or a placeholder while history
    /// is empty. A pick is kept, so it never changes under the user while settings stay open.
    pub(super) fn picked(&self, cx: &App) -> Sample {
        if let Some(picked) = self.picked.borrow().as_ref() {
            return picked.clone();
        }
        let history = Sonora::global(cx).history.read(cx);
        let tracks = history.tracks();
        let Some(track) =
            (!tracks.is_empty()).then(|| &tracks[(self.seed.get() % tracks.len() as u64) as usize])
        else {
            return Sample::placeholder();
        };
        let picked = Sample::of(track);
        self.picked.replace(Some(picked.clone()));
        picked
    }

    /// The ambient field following `cover`, for a preview to place.
    pub(super) fn ambient(&self, cover: Option<String>, cx: &mut App) -> Entity<Ambient> {
        self.ambient
            .update(cx, |ambient, cx| ambient.follow(cover, cx));
        self.ambient.clone()
    }
}

impl Sample {
    fn of(track: &music::Track) -> Self {
        Self {
            id: track.id.clone(),
            title: track.name.clone().into(),
            artist: track.artists.clone().into(),
            album: track.album.clone().into(),
            album_id: track.album_id.clone(),
            cover: track.cover.clone(),
            duration: track.duration,
        }
    }

    /// A neutral track for a preview while history holds none.
    fn placeholder() -> Self {
        Self {
            id: None,
            title: t!("settings-preview-title"),
            artist: t!("settings-preview-artist"),
            album: t!("settings-preview-album"),
            album_id: None,
            cover: None,
            duration: PLACEHOLDER_LENGTH,
        }
    }

    /// The largest cover the app knows for the track, the one it was listed with otherwise.
    pub(super) fn large_cover(&self, cx: &App) -> Option<String> {
        self.album_id
            .as_deref()
            .and_then(|album| Sonora::global(cx).cover.read(cx).large_for(album))
            .map(str::to_owned)
            .or_else(|| self.cover.clone())
    }
}

impl Tick {
    /// Redraws the settings view `wait` from now, or later while the window is in the
    /// background and battery saving caps its frames. A redraw already on its way stands.
    pub(super) fn after(&self, wait: Duration, window: &Window, cx: &mut Context<SettingsView>) {
        if self.pending.get() {
            return;
        }
        let saved = match window.is_window_active() {
            true => None,
            false => Sonora::global(cx).settings.read(cx).saver().interval(),
        };
        let wait = wait.max(saved.unwrap_or_default());
        self.pending.set(true);
        let pending = self.pending.clone();
        self.task.replace(Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            pending.set(false);
            this.update(cx, |_, cx| cx.notify()).ok();
        })));
    }
}

/// Frames a preview in the theme's radius with the border around it. GPUI clips only to a
/// rectangle, so a ring in the page colour, reaching a little past the frame, hides whatever
/// `content` paints into the corners outside the rounding. The ring paints over the frame's
/// neighbours by `CORNER_REACH` of the radius, which only ever lands on empty page.
pub(super) fn framed(content: impl IntoElement, cx: &App) -> Div {
    let theme = cx.theme();
    // a ring whose inner edge is the frame's rounded outline covers each corner once its outer
    // corner, rounded by the radius plus the reach, starts past the frame's own corner
    let reach = theme.radius * CORNER_REACH;
    let mut ring = div()
        .absolute()
        .top(-reach)
        .left(-reach)
        .right(-reach)
        .bottom(-reach)
        .rounded(theme.radius + reach)
        .border_color(theme.background);
    ring.style().border_widths = EdgesRefinement {
        top: Some(reach.into()),
        right: Some(reach.into()),
        bottom: Some(reach.into()),
        left: Some(reach.into()),
    };

    div()
        .relative()
        .w_full()
        .child(
            div()
                .w_full()
                .overflow_hidden()
                .rounded(theme.radius)
                .child(content),
        )
        .when(reach > Pixels::ZERO, |this| this.child(ring))
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border),
        )
}

/// A fresh random number from the standard library's own seeding.
fn random() -> u64 {
    RandomState::new().hash_one(std::time::Instant::now())
}
