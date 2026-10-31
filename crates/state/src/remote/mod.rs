use std::ffi::c_void;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use music::Track;
use tokio::sync::mpsc;

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
    Toggle,
    Next,
    Previous,
    Seek(Duration),
    Forward(Duration),
    Back(Duration),
    Volume(f64),
    /// Only MPRIS carries shuffle and repeat, souvlaki has neither.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    Shuffle(bool),
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    Repeat(Repeat),
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
    reported: Option<PlaybackState>,
    at: Duration,
    /// When `at` was published, so the next position can be checked against steady playback.
    stamp: Instant,
    volume: Option<f32>,
    shuffle: Option<bool>,
    repeat: Option<Repeat>,
    artwork: Option<Task<()>>,
    _events: Task<()>,
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
            reported: None,
            at: Duration::ZERO,
            stamp: Instant::now(),
            volume: None,
            shuffle: None,
            repeat: None,
            artwork: None,
            _events,
        }
    }

    fn act(&mut self, command: Command, cx: &mut Context<Self>) {
        self.playback
            .clone()
            .update(cx, |playback, cx| match command {
                Command::Play => playback.resume(cx),
                Command::Pause => playback.pause(cx),
                Command::Toggle => playback.toggle_play(cx),
                Command::Next => playback.next(cx),
                Command::Previous => playback.previous(cx),
                Command::Seek(at) => playback.seek(at, cx),
                Command::Forward(step) => {
                    shift(playback, playback.position().saturating_add(step), cx)
                }
                Command::Back(step) => {
                    shift(playback, playback.position().saturating_sub(step), cx)
                }
                Command::Volume(level) => playback.set_volume(level as f32, cx),
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                Command::Repeat(repeat) => playback.set_repeat(repeat, cx),
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                Command::Shuffle(on) => playback.set_shuffle(on, cx),
            });
    }

    fn publish(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        // Desktop media widgets follow the same device the player bar shows. In particular,
        // Spotify Connect playback can be active on a phone while the local engine is idle.
        let steered = playback.steered(cx);
        let state = match steered.as_ref() {
            Some(steered) => match steered.state.playing {
                true => PlaybackState::Playing,
                false => PlaybackState::Paused,
            },
            None => playback.state().clone(),
        };
        let (at, _) = playback.shown_time(cx);
        let track = playback.shown_track(cx);
        let volume = playback.shown_volume(cx);
        let repeat = playback.shown_repeat(cx);
        let shuffle = playback.shown_shuffle(cx);

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
                _ => self.controls.describe(track, cover.filter(|_| !remote), cx),
            }
            if let (Some(track), Some(url), true) = (track, cover, remote) {
                let (track, url) = (track.clone(), url.to_owned());
                self.artwork = Some(self.fetch_artwork(track, url, cx));
            }
        }

        if self.reported.as_ref() == Some(&state) && self.at.as_secs() == at.as_secs() {
            return;
        }
        let expected = match self.reported {
            Some(PlaybackState::Playing) => self.at.saturating_add(self.stamp.elapsed()),
            _ => self.at,
        };
        let jumped = !moved && at.abs_diff(expected) > SEEK_SLACK;
        let steady = !moved && !jumped && self.reported.as_ref() == Some(&state);
        self.reported = Some(state.clone());
        self.at = at;
        self.stamp = Instant::now();

        // macos runs the clock
        if cfg!(target_os = "macos") && steady {
            return;
        }
        self.controls.set_playback(&state, at);
        if jumped {
            self.controls.seeked(at);
        }
    }
}

impl Remote {
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
    /// download that fails there takes the process down.
    fn fetch_artwork(&self, track: Track, url: String, cx: &mut Context<Self>) -> Task<()> {
        let io = self.io.clone();
        cx.spawn(async move |this, cx| {
            let fetched = join(io.spawn(async move { artwork(&url).await })).await;
            let path = match fetched {
                Ok(path) => path,
                Err(error) => return log::debug!("remote: cannot fetch the cover: {error:#}"),
            };
            this.update(cx, |this, cx| {
                if this.shown == track.id {
                    let cover = format!("file://{}", path.display());
                    this.controls.describe(Some(&track), Some(&cover), cx);
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

/// Seeks to `target`, held inside the current track.
fn shift(playback: &mut Playback, target: Duration, cx: &mut Context<Playback>) {
    let end = playback
        .track()
        .map(|track| track.duration)
        .unwrap_or(target);
    playback.seek(target.min(end), cx);
}
