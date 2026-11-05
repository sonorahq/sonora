use std::ffi::c_void;
use std::time::Duration;

use anyhow::{Result, anyhow};
use gpui::App;
use music::Track;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use tokio::sync::mpsc;

use super::{BUS_NAME, Command, DISPLAY_NAME};
use crate::{PlaybackState, Repeat};

const SEEK_STEP: Duration = Duration::from_secs(5);

/// The macOS Now Playing center or the Windows transport controls, through `souvlaki`, which
/// has no shuffle, repeat or volume there.
pub struct Controls {
    controls: MediaControls,
}

impl Controls {
    pub fn new(
        hwnd: Option<*mut c_void>,
        commands: mpsc::UnboundedSender<Command>,
        _cx: &mut App,
    ) -> Result<Self> {
        let config = PlatformConfig {
            dbus_name: BUS_NAME,
            display_name: DISPLAY_NAME,
            hwnd,
        };
        let mut controls = MediaControls::new(config).map_err(|error| anyhow!("{error:?}"))?;
        if let Err(error) = controls.attach(move |event| {
            if let Some(command) = command(event) {
                commands.send(command).ok();
            }
        }) {
            log::warn!("remote: cannot listen for media keys: {error:?}");
        }
        Ok(Self { controls })
    }

    pub fn describe(&mut self, track: Option<&Track>, cover: Option<&str>, _cx: &App) {
        let metadata = match track {
            Some(track) => MediaMetadata {
                title: Some(&track.name),
                artist: Some(&track.artists),
                album: Some(&track.album),
                duration: Some(track.duration),
                cover_url: cover,
            },
            None => MediaMetadata::default(),
        };
        if let Err(error) = self.controls.set_metadata(metadata) {
            log::warn!("remote: cannot publish the current track: {error:?}");
        }
    }

    pub fn set_playback(&mut self, state: &PlaybackState, at: Duration) {
        let progress = Some(MediaPosition(at));
        let playback = match state {
            PlaybackState::Playing | PlaybackState::Loading => MediaPlayback::Playing { progress },
            PlaybackState::Paused => MediaPlayback::Paused { progress },
            PlaybackState::Idle | PlaybackState::Failed(_) => MediaPlayback::Stopped,
        };
        if let Err(error) = self.controls.set_playback(playback) {
            log::warn!("remote: cannot publish playback state: {error:?}");
        }
    }

    pub fn seeked(&mut self, _at: Duration) {}

    pub fn set_volume(&mut self, _level: f64) {}

    pub fn set_shuffle(&mut self, _on: bool) {}

    pub fn set_repeat(&mut self, _repeat: Repeat) {}
}

fn command(event: MediaControlEvent) -> Option<Command> {
    Some(match event {
        MediaControlEvent::Play => Command::Play,
        MediaControlEvent::Pause | MediaControlEvent::Stop => Command::Pause,
        MediaControlEvent::Toggle => Command::Toggle,
        MediaControlEvent::Next => Command::Next,
        MediaControlEvent::Previous => Command::Previous,
        MediaControlEvent::SetPosition(MediaPosition(at)) => Command::Seek(at),
        MediaControlEvent::Seek(direction) => step(direction, SEEK_STEP),
        MediaControlEvent::SeekBy(direction, by) => step(direction, by),
        MediaControlEvent::SetVolume(level) => Command::Volume(level),
        MediaControlEvent::OpenUri(_) | MediaControlEvent::Raise | MediaControlEvent::Quit => {
            return None;
        }
    })
}

fn step(direction: SeekDirection, by: Duration) -> Command {
    match direction {
        SeekDirection::Forward => Command::Forward(by),
        SeekDirection::Backward => Command::Back(by),
    }
}
