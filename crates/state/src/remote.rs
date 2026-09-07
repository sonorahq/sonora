use std::ffi::c_void;
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use tokio::sync::mpsc;

use crate::{Playback, PlaybackState, Sonora};

const BUS_NAME: &str = "sonora";
const DISPLAY_NAME: &str = "Sonora";

struct Attached {
    _remote: Entity<Remote>,
}

impl Global for Attached {}

pub fn attach(hwnd: Option<*mut c_void>, cx: &mut App) {
    if cx.has_global::<Attached>() {
        return;
    }
    let config = PlatformConfig {
        dbus_name: BUS_NAME,
        display_name: DISPLAY_NAME,
        hwnd,
    };
    let controls = match MediaControls::new(config) {
        Ok(controls) => controls,
        Err(error) => {
            return log::warn!("remote: cannot reach the system media controls: {error:?}");
        }
    };

    let playback = Sonora::global(cx).playback.clone();
    let remote = cx.new(|cx| Remote::new(controls, playback, cx));
    cx.set_global(Attached { _remote: remote });
}

pub struct Remote {
    controls: MediaControls,
    playback: Entity<Playback>,
    shown: Option<String>,
    reported: Option<PlaybackState>,
    at: Duration,
    _events: Task<()>,
}

impl Remote {
    fn new(
        mut controls: MediaControls,
        playback: Entity<Playback>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        if let Err(error) = controls.attach(move |event| {
            sender.send(event).ok();
        }) {
            log::warn!("remote: cannot listen for media keys: {error:?}");
        }

        let _events = cx.spawn(async move |this, cx| {
            while let Some(event) = receiver.recv().await {
                if this.update(cx, |this, cx| this.act(event, cx)).is_err() {
                    break;
                }
            }
        });

        cx.observe(&playback, |this, _, cx| this.publish(cx))
            .detach();

        Self {
            controls,
            playback,
            shown: None,
            reported: None,
            at: Duration::ZERO,
            _events,
        }
    }

    fn act(&mut self, event: MediaControlEvent, cx: &mut Context<Self>) {
        self.playback
            .clone()
            .update(cx, |playback, cx| match event {
                MediaControlEvent::Play => playback.resume(cx),
                MediaControlEvent::Pause | MediaControlEvent::Stop => playback.pause(cx),
                MediaControlEvent::Toggle => playback.toggle_play(cx),
                MediaControlEvent::Next => playback.next(cx),
                MediaControlEvent::Previous => playback.previous(cx),
                MediaControlEvent::SetPosition(MediaPosition(at)) => playback.seek(at, cx),
                MediaControlEvent::Seek(SeekDirection::Forward) => playback.seek_forward(cx),
                MediaControlEvent::Seek(SeekDirection::Backward) => playback.seek_back(cx),
                MediaControlEvent::SeekBy(SeekDirection::Forward, step) => {
                    playback.seek_by(step, true, cx)
                }
                MediaControlEvent::SeekBy(SeekDirection::Backward, step) => {
                    playback.seek_by(step, false, cx)
                }
                MediaControlEvent::SetVolume(level) => playback.set_volume(level as f32, cx),
                MediaControlEvent::OpenUri(_)
                | MediaControlEvent::Raise
                | MediaControlEvent::Quit => {}
            });
    }

    fn publish(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        let state = playback.state().clone();
        let at = playback.position();
        let track = playback.track().cloned();

        let id = track.as_ref().and_then(|track| track.id.clone());
        if id != self.shown {
            self.shown = id;
            let metadata = match &track {
                Some(track) => MediaMetadata {
                    title: Some(&track.name),
                    artist: Some(&track.artists),
                    album: Some(&track.album),
                    duration: Some(track.duration),
                    cover_url: track.cover.as_deref(),
                },
                None => MediaMetadata::default(),
            };
            if let Err(error) = self.controls.set_metadata(metadata) {
                log::warn!("remote: cannot publish the current track: {error:?}");
            }
        }

        if self.reported.as_ref() == Some(&state) && self.at.as_secs() == at.as_secs() {
            return;
        }
        self.reported = Some(state.clone());
        self.at = at;

        let progress = Some(MediaPosition(at));
        let reported = match state {
            PlaybackState::Playing | PlaybackState::Loading => MediaPlayback::Playing { progress },
            PlaybackState::Paused => MediaPlayback::Paused { progress },
            PlaybackState::Idle | PlaybackState::Failed(_) => MediaPlayback::Stopped,
        };
        if let Err(error) = self.controls.set_playback(reported) {
            log::warn!("remote: cannot publish playback state: {error:?}");
        }
    }
}
