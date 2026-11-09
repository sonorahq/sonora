use std::ffi::c_void;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result};
use music::{MediaKind, Track};

use crate::{Change, Cover, Cx, Job, PlaybackState, Repeat, Session, Shelf, Store, Topic};

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

/// What the player shows, as the media widget is to show it too: the device that has playback,
/// which is another one while it is steered.
struct Shown {
    state: PlaybackState,
    at: Duration,
    track: Option<Track>,
    volume: f32,
    repeat: Repeat,
    shuffle: bool,
}

/// The system media controls: what plays published to them, and their requests carried back.
pub struct Remote {
    controls: Controls,
    shown: Option<String>,
    source: Option<String>,
    /// The cover and length last handed to the widget, so a length that only arrives once
    /// the stream loads can be republished without dropping the cover already on show.
    art: Option<String>,
    length: Option<Duration>,
    reported: Option<PlaybackState>,
    at: Duration,
    /// When `at` was published, so the next position can be checked against steady playback.
    stamp: Instant,
    volume: Option<f32>,
    shuffle: Option<bool>,
    repeat: Option<Repeat>,
    artwork: Option<Job>,
}

impl Remote {
    fn new(controls: Controls) -> Self {
        Self {
            controls,
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
        }
    }

    fn publish(&mut self, shown: Shown, cover: &Cover, session: &Session, cx: &mut Cx) {
        let Shown {
            state,
            at,
            track,
            volume,
            repeat,
            shuffle,
        } = shown;

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
        let art = track.and_then(|track| artwork_url(track, cover));
        let moved = id != self.shown.as_deref();
        if moved || art != self.source.as_deref() {
            self.shown = id.map(str::to_owned);
            self.source = art.map(str::to_owned);
            self.artwork = None;
            let remote = art.is_some_and(is_remote);
            match (moved, remote) {
                // a remote cover follows once it sits in the cache, and a sharper one for the
                // track already on show leaves the published thumbnail up until the file lands
                (false, true) => {}
                // anything else is a file already
                _ => self.describe(track, art.filter(|_| !remote), session),
            }
            if let (Some(track), Some(url), true) = (track, art, remote) {
                let (track, url) = (track.clone(), url.to_owned());
                self.artwork = Some(fetch_artwork(track, url, cx));
            }
        }
        if track.map(|track| track.duration) != self.length {
            let art = self.art.take();
            self.describe(track, art.as_deref(), session);
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

    /// Hands the track to the widget and remembers what it was given.
    fn describe(&mut self, track: Option<&Track>, cover: Option<&str>, session: &Session) {
        let url = track.and_then(|track| track.id.as_deref()).and_then(|id| {
            session
                .client_of(Shelf::of(id))
                .and_then(|client| client.share_url(MediaKind::Track, id))
        });
        self.controls.describe(track, cover, url);
        self.art = cover.map(str::to_owned);
        self.length = track.map(|track| track.duration);
    }
}

impl Store {
    /// Publishes what plays to the system media controls and carries their requests back. The
    /// window handle is only read on Windows, where the controls hang off the window.
    pub fn attach_remote(&mut self, hwnd: Option<*mut c_void>) {
        if self.remote.is_some() {
            return;
        }
        let controls = match Controls::new(hwnd, self.cx.link()) {
            Ok(controls) => controls,
            Err(error) => {
                return log::warn!("remote: cannot reach the system media controls: {error:#}");
            }
        };
        self.remote = Some(Remote::new(controls));
        self.publish_remote();
    }

    /// Carries out what the desktop's media widget or media keys asked for.
    fn remote_act(&mut self, command: Command) {
        match command {
            Command::Play => self.resume_playback(),
            Command::Pause => self.pause_playback(),
            Command::Toggle => self.toggle_play(),
            Command::Next => self.next_track(),
            Command::Previous => self.previous_track(),
            Command::Seek(at) => self.seek_to(at),
            Command::Forward(step) => {
                let target = self.playback.position().saturating_add(step);
                self.shift(target)
            }
            Command::Back(step) => {
                let target = self.playback.position().saturating_sub(step);
                self.shift(target)
            }
            Command::Volume(level) => self.set_volume(level as f32),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            Command::Repeat(repeat) => self.set_repeat(repeat),
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            Command::Shuffle(on) => self.set_shuffle(on),
        }
    }

    /// Seeks to `target`, held inside the current track.
    fn shift(&mut self, target: Duration) {
        let end = self
            .playback
            .track()
            .map(|track| track.duration)
            .unwrap_or(target);
        self.seek_to(target.min(end));
    }

    /// What the player shows, read the way the player bar reads it. Desktop media widgets follow
    /// the same device the player bar shows. In particular, Spotify Connect playback can be
    /// active on a phone while the local engine is idle.
    fn remote_shown(&self) -> Shown {
        let Store {
            playback,
            queue,
            devices,
            ..
        } = self;
        let steered = playback.steered(devices);
        let state = match steered.as_ref() {
            Some(steered) => match steered.state.playing {
                true => PlaybackState::Playing,
                false => PlaybackState::Paused,
            },
            None => playback.state().clone(),
        };
        let (at, _) = playback.shown_time(devices);
        Shown {
            state,
            at,
            track: playback.shown_track(devices),
            volume: playback.shown_volume(devices),
            repeat: playback.shown_repeat(devices),
            shuffle: playback.shown_shuffle(queue, devices),
        }
    }

    fn publish_remote(&mut self) {
        if self.remote.is_none() {
            return;
        }
        let shown = self.remote_shown();
        let Store {
            remote,
            cover,
            session,
            cx,
            ..
        } = self;
        if let Some(remote) = remote {
            remote.publish(shown, cover, session, cx);
        }
    }

    pub(crate) fn react_remote(&mut self, change: &Change) {
        // the album art resolves after the track it belongs to, so republish when it lands
        if let Change::Topic(Topic::Playback | Topic::Queue | Topic::Cover) = change {
            self.publish_remote();
        }
    }
}

/// The cover to publish: the album art `Cover` resolves once it arrives, the thumbnail the
/// track carries until then. Spotify ships a 64px thumbnail with a track, which is plenty
/// for a list row and blurry in a desktop widget that draws it several times that size.
fn artwork_url<'a>(track: &'a Track, cover: &'a Cover) -> Option<&'a str> {
    track
        .album_id
        .as_deref()
        .and_then(|album| cover.large_for(album))
        .or(track.cover.as_deref())
}

/// Brings a remote cover into the cache and republishes the track with the file. The
/// platform widget would otherwise download it itself, on its own thread, and on macOS a
/// download that fails there takes the process down. The track is described as it stands
/// when the file lands, since its length may have arrived in the meantime.
fn fetch_artwork(track: Track, url: String, cx: &mut Cx) -> Job {
    cx.spawn(|link| async move {
        let path = match artwork(&url).await {
            Ok(path) => path,
            Err(error) => return log::debug!("remote: cannot fetch the cover: {error:#}"),
        };
        link.post(move |store| {
            let current = store.playback.shown_track(&store.devices);
            let Store {
                remote, session, ..
            } = store;
            let Some(remote) = remote else {
                return;
            };
            if let Some(current) = current.filter(|current| current.id == track.id)
                && remote.shown == track.id
            {
                let cover = format!("file://{}", path.display());
                remote.describe(Some(&current), Some(&cover), session);
            }
        });
    })
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
