use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use gpui::{App, AppContext as _, Context, Entity, Global};
use i18n::t;
use state::{PlaybackState, Sonora};

// Placeholder — swap for Sonora's real Discord Application ID once one exists.
const DISCORD_CLIENT_ID: &str = "PASTE_YOUR_DISCORD_APP_ID_HERE";

struct Installed {
    _discord: Entity<Discord>,
}

impl Global for Installed {}

pub fn install(cx: &mut App) {
    let discord = cx.new(Discord::new);
    cx.set_global(Installed { _discord: discord });
}

#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Off,
    Generic,
    Full {
        title: String,
        artist: String,
        album: String,
        cover: Option<String>,
        started_at_secs: u64,
    },
}

pub struct Discord {
    client: Option<DiscordIpcClient>,
    shown: Shown,
}

impl Discord {
    fn new(cx: &mut Context<Self>) -> Self {
        let playback = Sonora::global(cx).playback.clone();
        cx.observe(&playback, |this, _, cx| this.publish(cx))
            .detach();

        let settings = Sonora::global(cx).settings.clone();
        cx.observe(&settings, |this, _, cx| this.publish(cx))
            .detach();

        let client = DiscordIpcClient::new(DISCORD_CLIENT_ID);

        let mut this = Self {
            client: Some(client),
            shown: Shown::Off,
        };
        this.publish(cx);
        this
    }

    fn publish(&mut self, cx: &mut Context<Self>) {
        let shown = shown(cx);
        if shown == self.shown {
            return;
        }

        let Some(client) = self.client.as_mut() else {
            return;
        };

        // Small helper so a failed send can retry once after reconnecting,
        // without needing `activity::Activity` to implement Clone.
        let generic_text = t!("discord-listening-generic").to_string();

        match build_activity(&shown, &generic_text) {
            None => {
                let _ = client.clear_activity();
            }
            Some(payload) => {
                if client.set_activity(payload).is_err()
                    && client.connect().is_ok()
                    && let Some(retry) = build_activity(&shown, &generic_text)
                {
                    let _ = client.set_activity(retry);
                }
            }
        }

        self.shown = shown;
    }
}

fn build_activity<'a>(shown: &'a Shown, generic_text: &'a str) -> Option<activity::Activity<'a>> {
    match shown {
        Shown::Off => None,
        Shown::Generic => Some(
            activity::Activity::new()
                .activity_type(activity::ActivityType::Listening)
                .details(generic_text),
        ),
        Shown::Full {
            title,
            artist,
            album,
            cover,
            started_at_secs,
        } => {
            let mut assets = activity::Assets::new().large_text(album);
            if let Some(cover) = cover.as_deref() {
                assets = assets.large_image(cover);
            }
            Some(
                activity::Activity::new()
                    .activity_type(activity::ActivityType::Listening)
                    .details(title)
                    .state(artist)
                    .assets(assets)
                    .timestamps(activity::Timestamps::new().start(*started_at_secs as i64)),
            )
        }
    }
}

fn shown(cx: &App) -> Shown {
    let settings = Sonora::global(cx).settings.read(cx);
    if !settings.discord_rich_presence() {
        return Shown::Off;
    }

    let playback = Sonora::global(cx).playback.read(cx);
    let active = matches!(playback.state(), PlaybackState::Playing);
    let Some(track) = active.then(|| playback.track()).flatten() else {
        return Shown::Off;
    };

    if settings.discord_hide_details() {
        return Shown::Generic;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let position = playback.position();
    let started_at_secs = now.saturating_sub(position).as_secs();
    let cover = track.cover_large.clone().or_else(|| track.cover.clone());

    Shown::Full {
        title: track.name.clone(),
        artist: track.artists.clone(),
        album: track.album.clone(),
        cover,
        started_at_secs,
    }
}
