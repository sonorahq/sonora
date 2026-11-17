//! The main window's title on Windows. The shell shows that title in the taskbar hover preview,
//! the taskbar tooltip and Alt+Tab, so while a song plays it names the song and its artist.
//!
//! The title follows the track `Playback` shows, the same one the player bar and the thumbnail
//! toolbar read, so a Spotify Connect device counts too. It is written again only when the text
//! changes, and it goes back to the app name once playback stops.

use gpui::{AnyWindowHandle, App, Global, Subscription};
use music::Track;
use state::Sonora;

/// The name the window carries when nothing plays. It matches the title the window opens with,
/// which is untranslated too.
const APP_NAME: &str = "Sonora";

/// Holds the playback watch for the open window. Installing again for another window drops the
/// old watch, so only the window on screen gets retitled.
struct Installed {
    _watch: Subscription,
}

impl Global for Installed {}

/// Keeps the title of `window` in step with the track playback shows. Call it once per window
/// open, after the window exists, so a window reopened from the tray picks up the current song.
pub fn install(window: AnyWindowHandle, cx: &mut App) {
    let playback = Sonora::global(cx).playback.clone();
    let mut last = Last::default();
    // The window is still being built here, so the first write waits until the frame is up.
    cx.defer(move |cx| publish(window, &mut Last::default(), cx));
    let watch = cx.observe(&playback, move |_, cx| publish(window, &mut last, cx));
    cx.set_global(Installed { _watch: watch });
}

/// The title the main window carries for `track`: the song, its artist and the app name while a
/// song plays, and the app name alone otherwise. This is the one place the format lives.
pub fn title(track: Option<&Track>) -> String {
    match track {
        Some(track) if track.artists.is_empty() => format!("{} - {APP_NAME}", track.name),
        Some(track) => format!("{} - {} - {APP_NAME}", track.name, track.artists),
        None => APP_NAME.to_owned(),
    }
}

/// The title the window was last given, so a playback change that alters nothing visible costs
/// no window call.
#[derive(Default)]
struct Last(Option<String>);

impl Last {
    /// Returns the title for `track` when it differs from the one recorded, and records it.
    /// Returns `None` when the window already shows that title.
    fn change(&mut self, track: Option<&Track>) -> Option<String> {
        let title = title(track);
        if self.0.as_deref() == Some(title.as_str()) {
            return None;
        }
        self.0 = Some(title.clone());
        Some(title)
    }

    /// Forgets the recorded title, so the next change is written even when it matches.
    fn forget(&mut self) {
        self.0 = None;
    }
}

/// Writes the title for the track playback shows now, if it differs from the last one written.
/// A failed write forgets the recorded title so the next playback change tries again.
fn publish(window: AnyWindowHandle, last: &mut Last, cx: &mut App) {
    let track = Sonora::global(cx).playback.read(cx).shown_track(cx);
    let Some(title) = last.change(track.as_ref()) else {
        return;
    };
    if let Err(error) = window.update(cx, |_, window, _| window.set_window_title(&title)) {
        log::debug!("window title: cannot reach the main window: {error:#}");
        last.forget();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(name: &str, artists: &str) -> Track {
        Track {
            name: name.to_owned(),
            artists: artists.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn a_playing_song_shows_its_title_and_artist_before_the_app_name() {
        let track = track("Blue Monday", "New Order");
        assert_eq!(title(Some(&track)), "Blue Monday - New Order - Sonora");
    }

    #[test]
    fn a_song_without_an_artist_shows_its_title_and_the_app_name() {
        let track = track("Blue Monday", "");
        assert_eq!(title(Some(&track)), "Blue Monday - Sonora");
    }

    #[test]
    fn nothing_playing_shows_the_app_name() {
        assert_eq!(title(None), "Sonora");
    }

    #[test]
    fn the_title_follows_each_track_change_and_writes_only_when_it_differs() {
        let mut last = Last::default();
        let first = track("Blue Monday", "New Order");
        let second = track("Dreams", "Fleetwood Mac");

        assert_eq!(
            last.change(Some(&first)).as_deref(),
            Some("Blue Monday - New Order - Sonora"),
        );
        assert_eq!(last.change(Some(&first)), None);
        assert_eq!(
            last.change(Some(&second)).as_deref(),
            Some("Dreams - Fleetwood Mac - Sonora"),
        );
    }

    #[test]
    fn stopping_playback_drops_the_last_song_from_the_title() {
        let mut last = Last::default();
        let song = track("Blue Monday", "New Order");

        assert!(last.change(Some(&song)).is_some());
        assert_eq!(last.change(None).as_deref(), Some("Sonora"));
        assert_eq!(last.change(None), None);
    }

    #[test]
    fn a_forgotten_title_is_written_again_even_when_it_matches() {
        let mut last = Last::default();
        let song = track("Blue Monday", "New Order");

        assert!(last.change(Some(&song)).is_some());
        last.forget();
        assert_eq!(
            last.change(Some(&song)).as_deref(),
            Some("Blue Monday - New Order - Sonora"),
        );
    }
}
