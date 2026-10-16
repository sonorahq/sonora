use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::{DiscordIpc, activity};
use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use music::artwork::{ArtworkQuery, ArtworkSearch};
use music::{MediaKind, MusicProvider, Track};
use tokio::sync::watch;

use crate::{AppSettings, Cover, Devices, DiscordName, Io, Playback, Session, Shelf, join};

#[cfg(unix)]
mod socket;

#[cfg(not(unix))]
use discord_rich_presence::DiscordIpcClient as Client;
#[cfg(unix)]
use socket::Socket as Client;

const APPLICATION_ID: &str = "1547350467904806923";
const RETRY_DELAY: Duration = Duration::from_secs(3);
const SWITCH_DEBOUNCE: Duration = Duration::from_secs(1);
const MAX_START_DRIFT_SECONDS: u64 = 2;
const MAX_TEXT_UTF16_UNITS: usize = 128;
const MAX_IMAGE_UTF16_UNITS: usize = 300;
/// What the status says when it names the music rather than a service.
const MUSIC: &str = "Music";
/// Where the Sonora button sends a friend who presses it.
const SONORA_URL: &str = "https://sonorahq.org";

struct Attached {
    _discord: Entity<Discord>,
}

impl Global for Attached {}

pub(crate) fn attach(
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    session: Entity<Session>,
    cover: Entity<Cover>,
    devices: Entity<Devices>,
    io: Io,
    cx: &mut App,
) {
    let discord = cx.new(|cx| Discord::new(playback, settings, session, cover, devices, io, cx));
    cx.set_global(Attached { _discord: discord });
}

#[derive(Clone, Debug, Default, PartialEq)]
enum Shown {
    #[default]
    Off,
    On(Box<Presence>),
}

#[derive(Clone, Debug, PartialEq)]
struct Presence {
    source: Option<Source>,
    details: String,
    state: Option<String>,
    image: Option<String>,
    image_text: Option<String>,
    started_at: Option<i64>,
    ends_at: Option<i64>,
    buttons: Option<Vec<Button>>,
}

/// A link under the status. Discord shows at most two, and only to other people, so the user
/// never sees their own buttons.
#[derive(Clone, Debug, PartialEq)]
struct Button {
    label: String,
    url: String,
}

/// The provider a track came from, as the presence shows it. `badge` is the provider slug, which
/// doubles as the key of the image uploaded to the Discord application, and is left out when the
/// badge is turned off. `listening` is what the status calls itself and follows the setting;
/// `name` is the badge tooltip and is always the provider's own name.
#[derive(Clone, Debug, PartialEq)]
struct Source {
    badge: Option<&'static str>,
    listening: Option<String>,
    name: &'static str,
}

#[derive(Default)]
struct Timing {
    listening_since: Option<i64>,
    track: Option<String>,
    track_started_at: Option<i64>,
}

impl Timing {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn listening_since(&mut self) -> i64 {
        *self.listening_since.get_or_insert_with(unix_time)
    }

    fn track_started_at(&mut self, track: &Track, position: Duration) -> i64 {
        let measured = unix_time().saturating_sub(position.as_secs() as i64);
        let moved = self
            .track_started_at
            .is_none_or(|started_at| started_at.abs_diff(measured) > MAX_START_DRIFT_SECONDS);
        if moved || self.track.as_deref() != track.id.as_deref() {
            self.track = track.id.clone();
            self.track_started_at = Some(measured);
        }
        self.track_started_at.unwrap_or(measured)
    }
}

/// Watches playback and settings, and hands each new presence to the worker.
struct Discord {
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    session: Entity<Session>,
    cover: Entity<Cover>,
    devices: Entity<Devices>,
    sender: watch::Sender<Shown>,
    timing: Timing,
    io: Io,
    artwork: ArtworkSearch,
    /// Covers found by name, and the queries that found nothing, so neither is asked twice.
    found: HashMap<ArtworkQuery, Option<String>>,
    /// The query the last presence had no cover for, taken by `look_up` once it is published.
    wanted: Option<ArtworkQuery>,
    /// The query last sent to the search. A failed one is not retried until another was asked.
    asked: Option<ArtworkQuery>,
    lookup: Option<Task<()>>,
    _worker: Task<()>,
}

impl Discord {
    fn new(
        playback: Entity<Playback>,
        settings: Entity<AppSettings>,
        session: Entity<Session>,
        cover: Entity<Cover>,
        devices: Entity<Devices>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, receiver) = watch::channel(Shown::Off);
        let worker = io.spawn(run(receiver));
        let _worker = cx.spawn(async move |_, _| {
            if let Err(error) = worker.await {
                log::warn!("discord: the presence worker stopped: {error}");
            }
        });

        cx.observe(&playback, |this, _, cx| this.publish(cx))
            .detach();
        cx.observe(&settings, |this, _, cx| this.publish(cx))
            .detach();
        cx.observe(&cover, |this, _, cx| this.publish(cx)).detach();
        cx.observe(&devices, |this, _, cx| this.publish(cx))
            .detach();

        Self {
            playback,
            settings,
            session,
            cover,
            devices,
            sender,
            timing: Timing::default(),
            io,
            artwork: ArtworkSearch::new(),
            found: HashMap::new(),
            wanted: None,
            asked: None,
            lookup: None,
            _worker,
        }
    }

    fn publish(&mut self, cx: &mut Context<Self>) {
        let shown = self.shown(cx);
        self.sender.send_if_modified(|current| {
            let changed = *current != shown;
            if changed {
                *current = shown;
            }
            changed
        });
        self.look_up(cx);
    }

    /// Searches for the cover the last presence went without, then publishes again with it.
    /// Replacing the previous task throws away the answer for a track that is no longer playing.
    fn look_up(&mut self, cx: &mut Context<Self>) {
        let Some(query) = self.wanted.take() else {
            return;
        };
        if self.asked.as_ref() == Some(&query) {
            return;
        }
        self.asked = Some(query.clone());

        let search = self.artwork.clone();
        let io = self.io.clone();
        self.lookup = Some(cx.spawn(async move |this, cx| {
            let wanted = query.clone();
            let found = join(io.spawn(async move { search.find(&wanted).await })).await;

            this.update(cx, |this, cx| {
                this.lookup = None;
                match found {
                    Ok(cover) => {
                        this.found.insert(query, cover);
                        this.publish(cx);
                    }
                    Err(error) => log::warn!("discord: cannot look up the artwork: {error:#}"),
                }
            })
            .ok();
        }));
    }

    fn shown(&mut self, cx: &App) -> Shown {
        self.wanted = None;
        let playback = self.playback.read(cx);
        let Some(track) = playback.track() else {
            self.timing.reset();
            return Shown::Off;
        };
        // a paused status stays up when enabled, but without the timestamps, so nothing keeps counting
        let playing = playback.wants_playing();

        let since = self.timing.listening_since();
        let settings = self.settings.read(cx);
        if !settings.discord_presence() || !playing && !settings.discord_show_paused() {
            return Shown::Off;
        }
        let devices = self.devices.read(cx);
        if settings.discord_hide_on_connect()
            && (devices.publishing() || devices.steered().is_some())
        {
            return Shown::Off;
        }

        let session = self.session.read(cx);
        let provider = track.id.as_deref().and_then(|id| session.provider_for(id));
        let named = provider.map(|provider| Source {
            badge: settings.discord_badge().then(|| provider.slug()),
            listening: match settings.discord_name() {
                DiscordName::Sonora => None,
                DiscordName::Provider => Some(provider.listening_to().to_owned()),
                DiscordName::Music => Some(MUSIC.to_owned()),
                DiscordName::Title => fit_text(&track.name).or_else(|| Some(MUSIC.to_owned())),
                DiscordName::Artist => fit_text(&track.artists).or_else(|| Some(MUSIC.to_owned())),
                DiscordName::ArtistTitle => {
                    let artists = track.artists.trim();
                    let title = track.name.trim();
                    fit_text(&match (artists.is_empty(), title.is_empty()) {
                        (false, false) => format!("{artists} - {title}"),
                        (false, true) => artists.to_owned(),
                        (true, false) => title.to_owned(),
                        (true, true) => MUSIC.to_owned(),
                    })
                }
            },
            name: provider.name(),
        });
        if settings.discord_without_details() {
            return Shown::On(Box::new(Presence {
                source: named,
                details: anonymous_details(),
                state: None,
                image: None,
                image_text: None,
                started_at: playing.then_some(since),
                ends_at: None,
                buttons: buttons(settings, session, None),
            }));
        }

        let started_at = playing.then(|| {
            self.timing
                .track_started_at(track, playback.live_position())
        });
        let duration = track.duration.as_secs() as i64;
        let public_art = provider.is_some_and(|provider| provider.public_art());
        let lookup = !track.id.as_deref().is_some_and(music::is_local_id)
            || settings.artwork_for_local_files();
        Shown::On(Box::new(Presence {
            source: named,
            details: fit_text(&track.name).unwrap_or_else(anonymous_details),
            state: fit_text(&track.artists),
            image: self.artwork(track, public_art, lookup, cx),
            image_text: fit_text(&track.album),
            started_at,
            ends_at: started_at
                .filter(|_| duration > 0)
                .map(|started_at| started_at.saturating_add(duration)),
            buttons: buttons(settings, session, provider.zip(track.id.as_deref())),
        }))
    }

    /// Cover art for the track. The provider's own art is used only when it is public and within
    /// Discord's image length limit; URLs longer than the limit are skipped in favour of the name
    /// lookup. Without it, and when `lookup` allows, the cover is one found by name, and a search
    /// not yet made is left in `wanted` for `look_up`.
    fn artwork(
        &mut self,
        track: &Track,
        public_art: bool,
        lookup: bool,
        cx: &App,
    ) -> Option<String> {
        let own = public_art
            .then(|| {
                track
                    .album_id
                    .as_deref()
                    .and_then(|album| self.cover.read(cx).large_for(album))
                    .map(str::to_owned)
                    .or_else(|| track.cover.clone())
                    .filter(|cover| {
                        cover.starts_with("https://")
                            && cover.encode_utf16().count() <= MAX_IMAGE_UTF16_UNITS
                    })
            })
            .flatten();
        if own.is_some() || !lookup {
            return own;
        }
        let query = ArtworkQuery::for_track(track)?;
        match self.found.get(&query) {
            Some(found) => found.clone(),
            None => {
                self.wanted = Some(query);
                None
            }
        }
    }
}

/// Why a presence did not land. A refused payload is permanent; an unreachable socket is
/// worth trying again.
enum Failure {
    Unreachable(String),
    Refused(String),
}

async fn run(mut receiver: watch::Receiver<Shown>) {
    let mut client = Client::new(APPLICATION_ID);
    let mut shown = Shown::Off;
    let mut reported: Option<String> = None;

    while let Some(wanted) = next_presence(&mut receiver, &shown).await {
        // the socket blocks on both halves of a round trip, so it gets a thread of its own
        let attempted = tokio::task::spawn_blocking(move || {
            let result = send(&mut client, &wanted);
            (client, wanted, result)
        })
        .await;

        let Ok((used, wanted, result)) = attempted else {
            log::warn!("discord: the presence thread went away");
            return;
        };
        client = used;

        match result {
            Ok(()) => {
                shown = wanted;
                reported = None;
            }
            // Offering the same refused payload again would only reconnect forever, so it
            // counts as shown and waits for the next change.
            Err(Failure::Refused(message)) => {
                complain(&mut reported, message);
                shown = wanted;
            }
            Err(Failure::Unreachable(message)) => {
                complain(&mut reported, message);
                tokio::select! {
                    changed = receiver.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    () = tokio::time::sleep(RETRY_DELAY) => {}
                }
            }
        }
    }
}

/// Returns the next useful presence. Replacements are debounced until the listener settles on a
/// track, while turning presence off remains immediate.
async fn next_presence(receiver: &mut watch::Receiver<Shown>, shown: &Shown) -> Option<Shown> {
    loop {
        let wanted = receiver.borrow_and_update().clone();
        if &wanted == shown {
            receiver.changed().await.ok()?;
            continue;
        }
        if matches!(wanted, Shown::Off) {
            return Some(wanted);
        }

        tokio::select! {
            biased;

            changed = receiver.changed() => {
                changed.ok()?;
            }
            () = tokio::time::sleep(SWITCH_DEBOUNCE) => {
                return Some(wanted);
            }
        }
    }
}

/// Says what went wrong once, however long it goes on going wrong.
fn complain(reported: &mut Option<String>, failure: String) {
    if reported.as_ref() == Some(&failure) {
        return;
    }

    log::warn!("discord: cannot show the presence: {failure}");
    *reported = Some(failure);
}

/// Offers one presence to Discord, reconnecting once for a socket that has gone stale. The
/// client connects lazily, so the first offer of a session always takes this second path.
fn send(client: &mut Client, shown: &Shown) -> Result<(), Failure> {
    match offer(client, shown) {
        Err(Failure::Unreachable(_)) => {
            client
                .connect()
                .map_err(|error| Failure::Unreachable(error.to_string()))?;
            offer(client, shown)
        }
        result => result,
    }
}

fn offer(client: &mut Client, shown: &Shown) -> Result<(), Failure> {
    match activity(shown) {
        Some(activity) => client.set_activity(activity),
        None => client.clear_activity(),
    }
    .map_err(|error| Failure::Unreachable(error.to_string()))?;

    let (_, answer) = client
        .recv()
        .map_err(|error| Failure::Unreachable(error.to_string()))?;
    if answer.get("evt").and_then(|event| event.as_str()) != Some("ERROR") {
        return Ok(());
    }

    let message = answer
        .pointer("/data/message")
        .and_then(|message| message.as_str())
        .unwrap_or("discord turned the activity down");
    Err(Failure::Refused(message.to_owned()))
}

/// The activity to send, or nothing when the presence is to be cleared.
fn activity(shown: &Shown) -> Option<activity::Activity<'_>> {
    let Shown::On(presence) = shown else {
        return None;
    };

    let mut activity = activity::Activity::new()
        .activity_type(activity::ActivityType::Listening)
        .details(presence.details.as_str());
    if let Some(listening) = presence
        .source
        .as_ref()
        .and_then(|source| source.listening.as_deref())
    {
        activity = activity.name(listening);
    }
    if let Some(state) = presence.state.as_deref() {
        activity = activity.state(state);
    }
    if let Some(assets) = assets(presence) {
        activity = activity.assets(assets);
    }
    if let Some(buttons) = presence.buttons.as_deref() {
        let buttons = buttons
            .iter()
            .map(|button| activity::Button::new(button.label.as_str(), button.url.as_str()))
            .collect();
        activity = activity.buttons(buttons);
    }

    let Some(started_at) = presence.started_at else {
        return Some(activity);
    };

    let mut timestamps = activity::Timestamps::new().start(started_at);
    if let Some(ends_at) = presence.ends_at {
        timestamps = timestamps.end(ends_at);
    }
    Some(activity.timestamps(timestamps))
}

/// The artwork half of an activity, when there is any.
fn assets(presence: &Presence) -> Option<activity::Assets<'_>> {
    let image = presence.image.as_deref();
    let text = presence.image_text.as_deref();
    let badge = presence
        .source
        .as_ref()
        .and_then(|source| source.badge.map(|key| (key, source.name)));
    if image.is_none() && text.is_none() && badge.is_none() {
        return None;
    }

    let mut assets = activity::Assets::new();
    if let Some(image) = image {
        assets = assets.large_image(image);
    }
    if let Some(text) = text {
        assets = assets.large_text(text);
    }
    if let Some((key, name)) = badge {
        assets = assets.small_image(key).small_text(name);
    }
    Some(assets)
}

/// The buttons the settings ask for, or nothing when there are none. The provider button links
/// the track itself, so `track` is `None` when the details are hidden and the link would give
/// them away, and a provider without a public page for the track gets no button either.
fn buttons(
    settings: &AppSettings,
    session: &Session,
    track: Option<(&dyn MusicProvider, &str)>,
) -> Option<Vec<Button>> {
    let mut buttons = Vec::new();
    if settings.discord_provider_button()
        && let Some((provider, id)) = track
        && let shelf = Shelf::of(id)
        && !matches!(shelf, Shelf::Local)
        && let Some(url) = session
            .client_of(shelf)
            .and_then(|client| client.share_url(MediaKind::Track, id))
    {
        buttons.push(Button {
            label: format!("Listen on {}", provider.name()).to_string(),
            url,
        });
    }
    if settings.discord_sonora_button() {
        buttons.push(Button {
            label: "Get Sonora".to_string(),
            url: SONORA_URL.to_string(),
        });
    }
    (!buttons.is_empty()).then_some(buttons)
}

/// What the status says when the track is deliberately left out of it.
fn anonymous_details() -> String {
    let text = "Listening to music".to_string();
    fit_text(&text).unwrap_or(text)
}

/// Cuts a value to what Discord accepts in a text field, or drops it if nothing is left.
/// A lone character is padded with a non-breaking space, which Discord counts as the second
/// unit it insists on and draws as nothing.
fn fit_text(value: &str) -> Option<String> {
    let mut units = 0;
    let mut fitted = String::new();
    for character in value.trim().chars() {
        units += character.len_utf16();
        if units > MAX_TEXT_UTF16_UNITS {
            break;
        }
        fitted.push(character);
    }

    let mut fitted = fitted.trim_end().to_owned();
    match fitted.chars().count() {
        0 => None,
        1 => {
            fitted.push('\u{a0}');
            Some(fitted)
        }
        _ => Some(fitted),
    }
}

fn unix_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
