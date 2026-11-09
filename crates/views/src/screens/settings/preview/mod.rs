//! The previews of the pages where seeing a setting beats reading it. Each page with one has a
//! file here holding the preview's own state and the function that draws it, framed by
//! [`framed`]. The page lays it across its column, under its title and above its first group.
//!
//! Previews that move (the Now playing miniature's visualizer, the Discord card's clock) move
//! through Rune's live values from a [`Ticker`] thread that runs only while the preview is on
//! screen, so the view never runs for their frames.

pub(super) mod discord;
pub(super) mod lyrics;
pub(super) mod now_playing;
pub(super) mod sound;

use std::hash::{BuildHasher as _, RandomState};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use i18n::t;
use rune::{El, Transition, col, el};
use state::Store;
use ui::{Hsla, Theme};

use crate::Ui;

/// How long the placeholder track runs.
const PLACEHOLDER_LENGTH: Duration = Duration::from_secs(214);
/// How much of a preview shows while the feature it pictures is turned off.
pub(super) const DIMMED: f32 = 0.4;
const BORDER: f32 = 1.;
/// How strongly the ambient stand-in washes the field with the cover's colours, and how far its
/// glow reaches, as a share of the preview's width.
const WASH: f32 = 0.35;
const GLOW_REACH: f32 = 0.7;
const GLOW: f32 = 0.55;

/// The state every preview keeps between frames, one field per preview, each owned by its file,
/// and what they share.
pub struct Previews {
    pub(super) now_playing: now_playing::State,
    pub(super) sound: sound::State,
    pub(super) discord: discord::State,
    /// A random number drawn each time settings open, which picks the track from history.
    seed: u64,
    /// The track from history the previews show while nothing plays, kept from the moment it is
    /// picked until settings open again.
    picked: Option<Sample>,
}

impl Default for Previews {
    fn default() -> Self {
        Self {
            now_playing: now_playing::State::default(),
            sound: sound::State::default(),
            discord: discord::State::default(),
            seed: random(),
            picked: None,
        }
    }
}

/// The track a preview shows, as much of it as the previews draw.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Sample {
    pub(super) id: Option<String>,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) album: String,
    pub(super) album_id: Option<String>,
    pub(super) cover: Option<String>,
    pub(super) duration: Duration,
}

impl Previews {
    /// Settings opened: the next track from history is a new pick, made now when history is in.
    pub(crate) fn opened(&mut self, store: &Store) {
        self.seed = random();
        self.picked = pick(store, self.seed);
        self.discord.opened();
    }

    /// Settings closed: whatever a preview keeps running only for itself stops.
    pub(crate) fn closed(&mut self, store: &mut Store) {
        self.now_playing.hide();
        self.discord.hide();
        self.sound.hide(store);
    }
}

/// The track playing, or while nothing is loaded the one picked from history this time settings
/// opened, or a placeholder while history is empty.
pub(super) fn playing_or_picked(ui: &Ui) -> Sample {
    match ui.store.playback.track() {
        Some(track) => Sample::of(track),
        None => picked(ui),
    }
}

/// The track picked from history this time settings opened, or a placeholder while history is
/// empty. History still loading when settings opened is picked from by the same seed once it
/// is in.
pub(super) fn picked(ui: &Ui) -> Sample {
    let previews = &ui.screens.settings.previews;
    previews
        .picked
        .clone()
        .or_else(|| pick(&ui.store, previews.seed))
        .unwrap_or_else(Sample::placeholder)
}

fn pick(store: &Store, seed: u64) -> Option<Sample> {
    let tracks = store.history.tracks();
    (!tracks.is_empty()).then(|| Sample::of(&tracks[(seed % tracks.len() as u64) as usize]))
}

impl Sample {
    fn of(track: &music::Track) -> Self {
        Self {
            id: track.id.clone(),
            title: track.name.clone(),
            artist: track.artists.clone(),
            album: track.album.clone(),
            album_id: track.album_id.clone(),
            cover: track.cover.clone(),
            duration: track.duration,
        }
    }

    /// A neutral track for a preview while history holds none.
    fn placeholder() -> Self {
        Self {
            id: None,
            title: t!("settings-preview-title").into_owned(),
            artist: t!("settings-preview-artist").into_owned(),
            album: t!("settings-preview-album").into_owned(),
            album_id: None,
            cover: None,
            duration: PLACEHOLDER_LENGTH,
        }
    }

    /// The largest cover the app knows for the track, the one it was listed with otherwise.
    pub(super) fn large_cover(&self, store: &Store) -> Option<String> {
        self.album_id
            .as_deref()
            .and_then(|album| store.cover.large_for(album))
            .map(str::to_owned)
            .or_else(|| self.cover.clone())
    }
}

/// Frames a preview in the theme's radius with the border around it, clipping what `content`
/// paints into the corners. The frame fades in as the page shows it.
pub(super) fn framed(key: &str, content: El, theme: &Theme) -> El {
    col()
        .key(key)
        .w_full()
        .radius(theme.radius)
        .clip_rounded(theme.radius)
        .transition(Transition::opacity())
        .child(content)
        .child(
            el().key("frame-border")
                .absolute(0., 0.)
                .w_full()
                .h_full()
                .radius(theme.radius)
                .border(BORDER, theme.border.color())
                .inert()
                .a11y_hidden(),
        )
}

/// The colours a preview over the ambient field wears, and the field itself: the cover's own
/// colours washed over the background, glowing from the middle, standing in for the ambient
/// field Now playing paints until a preview can share that one. Without a palette yet, the
/// plain background and the theme.
pub(super) fn field(ui: &Ui, cover: Option<&str>, width: f32) -> (Theme, El) {
    let theme = &ui.theme;
    let palette = cover.and_then(|url| ui.artwork().cover_palette(url));
    let Some(primary) = palette.as_ref().and_then(|palette| palette.primary) else {
        return (
            *theme,
            el().key("field")
                .absolute(0., 0.)
                .w_full()
                .h_full()
                .fill(theme.background.color()),
        );
    };
    let secondary = palette
        .as_ref()
        .and_then(|palette| palette.secondary)
        .unwrap_or(primary);
    let tinted = theme.tinted(primary);
    let base = ui::mix(theme.background, primary, WASH);
    let glow: Hsla = ui::mix(base, secondary, GLOW);
    let field = el()
        .key("field")
        .absolute(0., 0.)
        .w_full()
        .h_full()
        .fill(base.color())
        .motion(ui::Springs::SLOW)
        .radial_gradient(
            [0.5, 0.35],
            [width * GLOW_REACH, width * GLOW_REACH / 2.],
            [(0., glow.color()), (1., glow.opacity(0.).color())],
        );
    (tinted, field)
}

/// A thread that runs `tick` every `period` while the preview it belongs to is on screen, for
/// a preview that moves through live values. Dropping or stopping it ends the thread.
#[derive(Default)]
pub(super) struct Ticker {
    running: Option<Arc<AtomicBool>>,
}

impl Ticker {
    /// Starts ticking unless already running.
    pub(super) fn start(&mut self, period: Duration, mut tick: impl FnMut() + Send + 'static) {
        if self.running.is_some() {
            return;
        }
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        let spawned = thread::Builder::new()
            .name("settings-preview".to_owned())
            .spawn(move || {
                while flag.load(Ordering::Relaxed) {
                    tick();
                    thread::sleep(period);
                }
            });
        match spawned {
            Ok(_) => self.running = Some(running),
            Err(error) => log::warn!("settings: cannot start a preview's ticker: {error}"),
        }
    }

    pub(super) fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.store(false, Ordering::Relaxed);
        }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A fresh random number from the standard library's own seeding.
pub(super) fn random() -> u64 {
    RandomState::new().hash_one(std::time::Instant::now())
}
