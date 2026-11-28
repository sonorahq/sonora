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
    /// The display of the same Windows transport controls `souvlaki` drives. Souvlaki keeps the
    /// last cover when a track arrives without one, so it is cleared here.
    #[cfg(target_os = "windows")]
    display: Option<windows::Media::SystemMediaTransportControlsDisplayUpdater>,
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
        Ok(Self {
            controls,
            #[cfg(target_os = "windows")]
            display: hwnd.and_then(|hwnd| match display(hwnd) {
                Ok(display) => Some(display),
                Err(error) => {
                    log::warn!("remote: cannot reach the transport controls display: {error}");
                    None
                }
            }),
        })
    }

    /// Publishes `track` with `cover`, a `file://` path or a url. Without a cover the widget shows
    /// none rather than the previous track's.
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
        #[cfg(target_os = "windows")]
        if metadata.cover_url.is_none()
            && let Some(display) = &self.display
            && let Err(error) = display.SetThumbnail(None)
        {
            log::warn!("remote: cannot clear the cover: {error}");
        }
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
        _ => return None,
    })
}

/// The display updater of the transport controls that belong to the window `hwnd`. Windows
/// hands out one set per window, so this is the one `souvlaki` updates.
#[cfg(target_os = "windows")]
fn display(
    hwnd: *mut c_void,
) -> windows::core::Result<windows::Media::SystemMediaTransportControlsDisplayUpdater> {
    use windows::Media::SystemMediaTransportControls;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

    let interop = windows::core::factory::<
        SystemMediaTransportControls,
        ISystemMediaTransportControlsInterop,
    >()?;
    let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd)) }?;
    controls.DisplayUpdater()
}

fn step(direction: SeekDirection, by: Duration) -> Command {
    match direction {
        SeekDirection::Forward => Command::Forward(by),
        SeekDirection::Backward => Command::Back(by),
    }
}
