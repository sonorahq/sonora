use std::ffi::c_void;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use music::Track;
use tokio::sync::mpsc;

use crate::playback::PlaybackEvent;
use crate::{Cover, Io, Playback, PlaybackState, Queue, Repeat, Sonora, join};

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod mpris;
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
mod souvlaki;

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
use self::souvlaki::Controls;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use mpris::Controls;

const BUS_NAME: &str = "sonora";
const DISPLAY_NAME: &str = "Sonora";
/// How far the position may stray from where steady playback would have put it before the
/// widget is told the track was seeked.
const SEEK_SLACK: Duration = Duration::from_secs(2);
const ARTWORK: &str = "artwork";
/// How many covers the artwork folder keeps. The widget shows only the current track's, so the
/// rest just save a download when the listener goes back.
const KEPT_COVERS: usize = 32;

struct Attached {
    _remote: Entity<Remote>,
}

impl Global for Attached {}

/// A request from the desktop's media widget or media keys, in the terms `Playback` and
/// `Queue` understand.
enum Command {
    Play,
    Pause,
    Stop,
    Toggle,
    Next,
    Previous,
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    Seek(Duration),
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    SetPosition {
        track_id: zbus::ObjectPath<'static>,
        at: Duration,
    },
    Forward(Duration),
    Back(Duration),
    Volume(f64),
    /// Only MPRIS carries shuffle and repeat, souvlaki has neither.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    Shuffle(bool),
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    Repeat(Repeat),
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    Raise,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    Quit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Capabilities {
    pub can_play: bool,
    pub can_pause: bool,
    pub can_seek: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReportedState {
    state: PlaybackState,
    wants_playing: bool,
    is_seeking: bool,
}

fn reported_state(playback: &Playback, cx: &App) -> ReportedState {
    let steered = playback.steered(cx);
    let state = match steered.as_ref() {
        Some(steered) => match steered.state.playing {
            true => PlaybackState::Playing,
            false => PlaybackState::Paused,
        },
        None => playback.state().clone(),
    };
    let wants_playing = match steered.as_ref() {
        Some(steered) => steered.state.playing,
        None => playback.wants_playing(),
    };
    let is_seeking = playback.is_seeking();
    ReportedState {
        state,
        wants_playing,
        is_seeking,
    }
}

/// Publishes what plays to the system media controls and carries their requests back. The
/// window handle is only read on Windows, where the controls hang off the window.
pub fn attach(hwnd: Option<*mut c_void>, cx: &mut App) {
    if cx.has_global::<Attached>() {
        return;
    }
    let (sender, receiver) = mpsc::unbounded_channel();
    let controls = match Controls::new(hwnd, sender, cx) {
        Ok(controls) => controls,
        Err(error) => {
            return log::warn!("remote: cannot reach the system media controls: {error:#}");
        }
    };

    let sonora = Sonora::global(cx);
    let playback = sonora.playback.clone();
    let queue = sonora.queue.clone();
    let cover = sonora.cover.clone();
    let io = Io::global(cx);
    let remote = cx.new(|cx| Remote::new(controls, receiver, playback, queue, cover, io, cx));
    remote.update(cx, |remote, cx| remote.publish(cx));
    cx.set_global(Attached { _remote: remote });
}

pub struct Remote {
    controls: Controls,
    playback: Entity<Playback>,
    cover: Entity<Cover>,
    io: Io,
    shown: Option<String>,
    source: Option<String>,
    /// The cover and length last handed to the widget, so a length that only arrives once
    /// the stream loads can be republished without dropping the cover already on show.
    art: Option<String>,
    length: Option<Duration>,
    reported: Option<ReportedState>,
    at: Duration,
    stamp: Instant,
    volume: Option<f32>,
    shuffle: Option<bool>,
    repeat: Option<Repeat>,
    artwork: Option<Task<()>>,
    _events: Task<()>,
    pending_seek: bool,
    capabilities: Option<Capabilities>,
}

impl Remote {
    fn new(
        controls: Controls,
        mut receiver: mpsc::UnboundedReceiver<Command>,
        playback: Entity<Playback>,
        queue: Entity<Queue>,
        cover: Entity<Cover>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        let _events = cx.spawn(async move |this, cx| {
            while let Some(command) = receiver.recv().await {
                if this.update(cx, |this, cx| this.act(command, cx)).is_err() {
                    break;
                }
            }
        });

        cx.observe(&playback, |this, _, cx| this.publish(cx))
            .detach();
        // the media controls show the position as it moves
        let ticks = playback.read(cx).ticks().clone();
        cx.observe(&ticks, |this, _, cx| this.publish(cx)).detach();
        cx.subscribe(&playback, |this, _, event, cx| {
            if let PlaybackEvent::Seeked(at) = event {
                this.on_seeked(*at, cx);
            }
        })
        .detach();
        cx.observe(&queue, |this, _, cx| this.publish(cx)).detach();
        // the album art resolves after the track it belongs to, so republish when it lands
        cx.observe(&cover, |this, _, cx| this.publish(cx)).detach();

        Self {
            controls,
            playback,
            cover,
            io,
            shown: None,
            source: None,
            art: None,
            length: None,
            reported: None,
            at: Duration::ZERO,
            stamp: Instant::now(),
            volume: None,
            shuffle: None,
            repeat: None,
            artwork: None,
            _events,
            pending_seek: false,
            capabilities: None,
        }
    }

    fn on_seeked(&mut self, at: Duration, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        let current = reported_state(playback, cx);
        self.pending_seek =
            matches!(current.state, PlaybackState::Loading) && current.wants_playing;
        self.reported = Some(current.clone());
        self.at = at;
        self.stamp = Instant::now();
        self.controls.set_playback(
            &current.state,
            at,
            current.wants_playing,
            current.is_seeking,
        );
        self.controls.seeked(at);
    }

    fn act(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            Command::Raise => {
                for window in cx.windows() {
                    let _ = window.update(cx, |_, window, _| {
                        window.activate_window();
                    });
                }
                cx.activate(true);
            }
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            Command::Quit => cx.quit(),
            command => {
                self.playback.update(cx, |playback, cx| match command {
                    Command::Play => playback.resume(cx),
                    Command::Pause => playback.pause(cx),
                    Command::Stop => {
                        if playback.shown_track(cx).is_some() {
                            playback.pause(cx);
                            playback.seek(Duration::ZERO, cx);
                        }
                    }
                    Command::Toggle => playback.toggle_play(cx),
                    Command::Next => playback.next(cx),
                    Command::Previous => playback.previous(cx),
                    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
                    Command::Seek(at) => playback.seek(at, cx),
                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    Command::SetPosition { track_id, at } => {
                        let (_, total) = playback.shown_time(cx);
                        if !total.is_zero()
                            && at <= total
                            && playback
                                .shown_track(cx)
                                .is_some_and(|track| mpris::track_id(&track) == track_id)
                        {
                            playback.seek(at, cx);
                        }
                    }
                    Command::Forward(step) => {
                        let (at, total) = playback.seek_origin(cx);
                        if total.is_zero() {
                            return;
                        }
                        let target = at.saturating_add(step);
                        let wrap_next = cfg!(any(target_os = "linux", target_os = "freebsd"));
                        match wrap_next && target > total {
                            true => playback.next(cx),
                            false => playback.seek(target.min(total), cx),
                        }
                    }
                    Command::Back(step) => {
                        let (at, total) = playback.seek_origin(cx);
                        if total.is_zero() {
                            return;
                        }
                        playback.seek(at.saturating_sub(step), cx);
                    }
                    Command::Volume(level) => playback.set_volume(level as f32, cx),
                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    Command::Repeat(repeat) => playback.set_repeat(repeat, cx),
                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    Command::Shuffle(on) => playback.set_shuffle(on, cx),
                    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                    Command::Raise | Command::Quit => {}
                });
            }
        }
    }

    fn publish(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        let current = reported_state(playback, cx);
        let steered = playback.steered(cx);
        let (shown, total) = playback.shown_time(cx);
        let at = match steered.is_some() {
            true => shown,
            false => playback.live_position(),
        };
        let track = playback.shown_track(cx);
        let volume = playback.shown_volume(cx);
        let repeat = playback.shown_repeat(cx);
        let shuffle = playback.shown_shuffle(cx);

        let queue_view = playback.shown_queue(cx);
        let can_play = track.is_some();
        let can_pause = track.is_some();
        let can_seek = !total.is_zero();
        let can_go_next = queue_view.sections.upcoming > 0
            || (track.is_some() && (repeat == Repeat::All || steered.is_some()));
        let can_go_previous = (track.is_some() && steered.is_some()) || playback.has_previous(cx);

        let capabilities = Capabilities {
            can_play,
            can_pause,
            can_seek,
            can_go_next,
            can_go_previous,
        };
        if self.capabilities != Some(capabilities) {
            self.capabilities = Some(capabilities);
            self.controls.set_capabilities(capabilities);
        }

        if self.volume != Some(volume) {
            self.volume = Some(volume);
            self.controls.set_volume(volume.into());
        }
        if self.shuffle != Some(shuffle) {
            self.shuffle = Some(shuffle);
            self.controls.set_shuffle(shuffle);
        }
        if self.repeat != Some(repeat) {
            self.repeat = Some(repeat);
            self.controls.set_repeat(repeat);
        }

        let track = track.as_ref();
        let id = track.and_then(|track| track.id.as_deref());
        let cover = track.and_then(|track| self.artwork_url(track, cx));
        let moved = id != self.shown.as_deref();
        if moved {
            self.pending_seek = false;
        }
        if moved || cover != self.source.as_deref() {
            self.shown = id.map(str::to_owned);
            self.source = cover.map(str::to_owned);
            self.artwork = None;
            let remote = cover.is_some_and(is_remote);
            match (moved, remote) {
                // a remote cover follows once it sits in the cache, and a sharper one for the
                // track already on show leaves the published thumbnail up until the file lands
                (false, true) => {}
                // anything else is a file already
                _ => self.describe(track, cover.filter(|_| !remote), cx),
            }
            if let (Some(track), Some(url), true) = (track, cover, remote) {
                let (track, url) = (track.clone(), url.to_owned());
                self.artwork = Some(self.fetch_artwork(track, url, cx));
            }
        }
        if self.length != Some(total) {
            let art = self.art.take();
            self.describe(track, art.as_deref(), cx);
        }

        if matches!(current.state, PlaybackState::Loading) && current.wants_playing {
            self.pending_seek = true;
        }

        let expected = match self.reported.as_ref() {
            Some(r) if r.state == PlaybackState::Playing && !r.is_seeking => {
                self.at.saturating_add(self.stamp.elapsed())
            }
            _ => self.at,
        };
        let jumped = !moved
            && !current.is_seeking
            && self.shown.is_some()
            && current.state != PlaybackState::Loading
            && at.abs_diff(expected) > SEEK_SLACK;
        let resync = !moved && self.pending_seek && current.state == PlaybackState::Playing;
        if resync {
            self.pending_seek = false;
        }
        let status_changed = self.reported.as_ref() != Some(&current);

        // macos runs the clock
        if cfg!(target_os = "macos") && !status_changed && !moved && !jumped && !resync {
            return;
        }

        if !status_changed && !moved && !jumped && !resync && self.at.as_secs() == at.as_secs() {
            return;
        }
        self.reported = Some(current.clone());
        self.at = at;
        self.stamp = Instant::now();

        self.controls.set_playback(
            &current.state,
            at,
            current.wants_playing,
            current.is_seeking,
        );
        if jumped || resync {
            self.controls.seeked(at);
        }
    }
}

impl Remote {
    /// Hands the track to the widget and remembers what it was given.
    fn describe(&mut self, track: Option<&Track>, cover: Option<&str>, cx: &App) {
        let total = self.playback.read(cx).shown_time(cx).1;
        self.controls.describe(track, cover, total, cx);
        self.art = cover.map(str::to_owned);
        self.length = Some(total);
    }

    /// The cover to publish: the album art `Cover` resolves once it arrives, the thumbnail the
    /// track carries until then. Spotify ships a 64px thumbnail with a track, which is plenty
    /// for a list row and blurry in a desktop widget that draws it several times that size.
    fn artwork_url<'a>(&self, track: &'a Track, cx: &'a App) -> Option<&'a str> {
        track
            .album_id
            .as_deref()
            .and_then(|album| self.cover.read(cx).large_for(album))
            .or(track.cover.as_deref())
    }

    /// Brings a remote cover into the cache and republishes the track with the file. The
    /// platform widget would otherwise download it itself, on its own thread, and on macOS a
    /// download that fails there takes the process down. The track is described as it stands
    /// when the file lands, since its length may have arrived in the meantime.
    fn fetch_artwork(&self, track: Track, url: String, cx: &mut Context<Self>) -> Task<()> {
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            let fetched = join(io.spawn(async move { artwork(&url).await })).await;
            let path = match fetched {
                Ok(path) => path,
                Err(error) => return log::debug!("remote: cannot fetch the cover: {error:#}"),
            };
            this.update(cx, |this, cx| {
                let current = this.playback.read(cx).shown_track(cx);
                if let Some(current) = current.filter(|current| current.id == track.id)
                    && this.shown == track.id
                {
                    let cover = format!("file://{}", path.display());
                    this.describe(Some(&current), Some(&cover), cx);
                }
            })
            .ok();
        })
    }
}

fn is_remote(cover: &str) -> bool {
    cover.starts_with("http://") || cover.starts_with("https://")
}

/// The cached file for a cover url, downloaded on first sight. The bytes have to decode as an
/// image before they are kept, so the widget never opens something it cannot draw.
async fn artwork(url: &str) -> Result<PathBuf> {
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora")
        .join(ARTWORK);
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    let key = format!("{:016x}", hasher.finish());
    for format in [
        image::ImageFormat::Jpeg,
        image::ImageFormat::Png,
        image::ImageFormat::WebP,
    ] {
        let candidate = dir.join(&key).with_extension(extension(format));
        if candidate.is_file() {
            std::fs::File::options()
                .write(true)
                .open(&candidate)
                .and_then(|file| file.set_modified(SystemTime::now()))
                .ok();
            return Ok(candidate);
        }
    }

    let bytes = reqwest::get(url)
        .await
        .context("cannot request the cover")?
        .error_for_status()
        .context("the cover request was refused")?
        .bytes()
        .await
        .context("cannot read the cover")?;
    let format = image::guess_format(&bytes).context("cannot tell the cover format")?;
    image::load_from_memory_with_format(&bytes, format).context("cannot decode the cover")?;

    std::fs::create_dir_all(&dir).context("cannot create the artwork cache")?;
    let path = dir.join(&key).with_extension(extension(format));
    std::fs::write(&path, &bytes).context("cannot store the cover")?;
    trim(&dir);
    Ok(path)
}

/// Deletes all but the [`KEPT_COVERS`] most recently used covers in `dir`. A cover the widget
/// still holds open on Windows fails to delete and goes on a later pass.
fn trim(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut covers: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry
                .metadata()
                .ok()
                .filter(|metadata| metadata.is_file())?;
            Some((metadata.modified().ok()?, entry.path()))
        })
        .collect();
    if covers.len() <= KEPT_COVERS {
        return;
    }
    covers.sort_unstable_by_key(|(used, _)| std::cmp::Reverse(*used));
    for (_, path) in covers.into_iter().skip(KEPT_COVERS) {
        std::fs::remove_file(path).ok();
    }
}

fn extension(format: image::ImageFormat) -> &'static str {
    format.extensions_str().first().copied().unwrap_or("img")
}
