use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui::{App, Context, Entity, SharedString, Task};
use music::audioscrobbler::{AudioScrobblerClient, LASTFM, LIBREFM};
use music::listenbrainz::ListenBrainzClient;
use music::{Scrobbler, Track};

use crate::playback::PlaybackEvent;
use crate::{AppSettings, Io, Outcome, Playback, Toasts, join};

const MIN_SCROBBLE_DURATION: Duration = Duration::from_secs(30);
const MAX_SCROBBLE_WAIT: Duration = Duration::from_secs(4 * 60);

pub struct Scrobbling {
    settings: Entity<AppSettings>,
    playback: Entity<Playback>,
    io: Io,
    lastfm: Option<Arc<AudioScrobblerClient>>,
    librefm: Option<Arc<AudioScrobblerClient>>,
    listenbrainz: Option<Arc<ListenBrainzClient>>,
    active: Option<String>,
    started_at: Option<SystemTime>,
    fired: bool,
    pending_lastfm_token: Option<String>,
    pending_librefm_token: Option<String>,
    lastfm_task: Option<Task<()>>,
    librefm_task: Option<Task<()>>,
    listenbrainz_task: Option<Task<()>>,
}

impl Scrobbling {
    pub fn new(
        settings: Entity<AppSettings>,
        playback: Entity<Playback>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&playback, |this, _, event, cx| match event {
            PlaybackEvent::StartedPlayback => this.start(cx),
            PlaybackEvent::EndedPlayback => this.stop(),
        })
        .detach();
        cx.observe(&playback, |this, _, cx| this.check_threshold(cx))
            .detach();
        cx.observe(&settings, |this, _, cx| this.rebuild(cx))
            .detach();

        let mut scrobbling = Self {
            settings,
            playback,
            io,
            lastfm: None,
            librefm: None,
            listenbrainz: None,
            active: None,
            started_at: None,
            fired: false,
            pending_lastfm_token: None,
            pending_librefm_token: None,
            lastfm_task: None,
            librefm_task: None,
            listenbrainz_task: None,
        };
        scrobbling.rebuild(cx);
        scrobbling
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let (
            lastfm_api_key,
            lastfm_api_secret,
            lastfm_session_key,
            librefm_api_key,
            librefm_api_secret,
            librefm_session_key,
            listenbrainz_token,
        ) = {
            let settings = self.settings.read(cx);
            (
                settings.lastfm_api_key().to_owned(),
                settings.lastfm_api_secret().to_owned(),
                settings.lastfm_session_key().to_owned(),
                settings.librefm_api_key().to_owned(),
                settings.librefm_api_secret().to_owned(),
                settings.librefm_session_key().to_owned(),
                settings.listenbrainz_token().to_owned(),
            )
        };
        self.lastfm = match lastfm_api_key.is_empty()
            || lastfm_api_secret.is_empty()
            || lastfm_session_key.is_empty()
        {
            true => None,
            false => Some(Arc::new(AudioScrobblerClient::new(
                &LASTFM,
                lastfm_api_key,
                lastfm_api_secret,
                lastfm_session_key,
            ))),
        };
        self.librefm = match librefm_api_key.is_empty()
            || librefm_api_secret.is_empty()
            || librefm_session_key.is_empty()
        {
            true => None,
            false => Some(Arc::new(AudioScrobblerClient::new(
                &LIBREFM,
                librefm_api_key,
                librefm_api_secret,
                librefm_session_key,
            ))),
        };
        self.listenbrainz = match listenbrainz_token.is_empty() {
            true => None,
            false => Some(Arc::new(ListenBrainzClient::new(listenbrainz_token))),
        };
    }

    fn scrobblers(&self) -> Vec<Arc<dyn Scrobbler>> {
        let mut list: Vec<Arc<dyn Scrobbler>> = Vec::new();
        if let Some(client) = &self.lastfm {
            list.push(client.clone());
        }
        if let Some(client) = &self.librefm {
            list.push(client.clone());
        }
        if let Some(client) = &self.listenbrainz {
            list.push(client.clone());
        }
        list
    }

    fn key_for(track: &Track) -> String {
        match &track.id {
            Some(id) => id.clone(),
            None => format!("{}\u{1}{}\u{1}{}", track.name, track.artists, track.album),
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(track) = self.playback.read(cx).track().cloned() else {
            return;
        };
        let key = Self::key_for(&track);
        if self.active.as_ref() == Some(&key) {
            return;
        }
        self.active = Some(key);
        self.started_at = Some(SystemTime::now());
        self.fired = false;

        for scrobbler in self.scrobblers() {
            let track = track.clone();
            self.io.spawn(async move {
                if let Err(error) = scrobbler.now_playing(&track).await {
                    log::warn!("scrobble: cannot update now playing: {error:#}");
                }
            });
        }
    }

    fn stop(&mut self) {
        self.active = None;
        self.started_at = None;
        self.fired = false;
    }

    fn check_threshold(&mut self, cx: &mut Context<Self>) {
        if self.fired || self.active.is_none() {
            return;
        }
        let Some(track) = self.playback.read(cx).track().cloned() else {
            return;
        };
        if track.duration < MIN_SCROBBLE_DURATION {
            return;
        }
        let position = self.playback.read(cx).position();
        let threshold = (track.duration / 2).min(MAX_SCROBBLE_WAIT);
        if position < threshold {
            return;
        }
        let Some(started_at) = self.started_at else {
            return;
        };
        self.fired = true;

        for scrobbler in self.scrobblers() {
            let track = track.clone();
            self.io.spawn(async move {
                if let Err(error) = scrobbler.scrobble(&track, started_at).await {
                    log::warn!("scrobble: cannot submit a scrobble: {error:#}");
                }
            });
        }
    }

    pub fn lastfm_username(&self, cx: &App) -> Option<SharedString> {
        let username = self.settings.read(cx).lastfm_username().to_owned();
        (!username.is_empty()).then(|| SharedString::from(username))
    }

    pub fn lastfm_api_key(&self, cx: &App) -> String {
        self.settings.read(cx).lastfm_api_key().to_owned()
    }

    pub fn lastfm_api_secret(&self, cx: &App) -> String {
        self.settings.read(cx).lastfm_api_secret().to_owned()
    }

    pub fn lastfm_awaiting_confirmation(&self) -> bool {
        self.pending_lastfm_token.is_some()
    }

    pub fn librefm_username(&self, cx: &App) -> Option<SharedString> {
        let username = self.settings.read(cx).librefm_username().to_owned();
        (!username.is_empty()).then(|| SharedString::from(username))
    }

    pub fn librefm_api_key(&self, cx: &App) -> String {
        self.settings.read(cx).librefm_api_key().to_owned()
    }

    pub fn librefm_api_secret(&self, cx: &App) -> String {
        self.settings.read(cx).librefm_api_secret().to_owned()
    }

    pub fn librefm_awaiting_confirmation(&self) -> bool {
        self.pending_librefm_token.is_some()
    }

    pub fn listenbrainz_connected(&self, cx: &App) -> bool {
        !self.settings.read(cx).listenbrainz_token().is_empty()
    }

    pub fn connect_lastfm(&mut self, api_key: String, api_secret: String, cx: &mut Context<Self>) {
        if self.lastfm_task.is_some() {
            return;
        }
        self.settings.update(cx, |settings, cx| {
            settings.set_lastfm_api_key(api_key.clone(), cx);
            settings.set_lastfm_api_secret(api_secret.clone(), cx);
        });
        let io = self.io.clone();
        self.lastfm_task = Some(cx.spawn(async move |this, cx| {
            let requested = join(io.spawn(async move {
                music::audioscrobbler::request_token(&LASTFM, &api_key, &api_secret).await
            }))
            .await;
            this.update(cx, |this, cx| {
                this.lastfm_task = None;
                match requested {
                    Ok(token) => {
                        let api_key = this.settings.read(cx).lastfm_api_key();
                        let url = music::audioscrobbler::auth_url(&LASTFM, api_key, &token);
                        this.pending_lastfm_token = Some(token);
                        cx.open_url(&url);
                    }
                    Err(error) => {
                        log::warn!("scrobble: cannot request a last.fm token: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-lastfm-failed", cx);
                    }
                }
            })
            .ok();
        }));
    }

    pub fn confirm_lastfm(&mut self, cx: &mut Context<Self>) {
        if self.lastfm_task.is_some() {
            return;
        }
        let Some(token) = self.pending_lastfm_token.clone() else {
            return;
        };
        let (api_key, api_secret) = {
            let settings = self.settings.read(cx);
            (
                settings.lastfm_api_key().to_owned(),
                settings.lastfm_api_secret().to_owned(),
            )
        };
        let io = self.io.clone();
        self.lastfm_task = Some(cx.spawn(async move |this, cx| {
            let exchanged = join(io.spawn(async move {
                music::audioscrobbler::exchange_session(&LASTFM, &api_key, &api_secret, &token)
                    .await
            }))
            .await;
            this.update(cx, |this, cx| {
                this.lastfm_task = None;
                this.pending_lastfm_token = None;
                match exchanged {
                    Ok((session_key, username)) => {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_lastfm_session_key(session_key, cx);
                            settings.set_lastfm_username(username.clone(), cx);
                        });
                        Toasts::about(Outcome::Done, "toast-lastfm-connected", username, cx);
                    }
                    Err(error) => {
                        log::warn!("scrobble: cannot complete last.fm sign-in: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-lastfm-failed", cx);
                    }
                }
            })
            .ok();
        }));
    }

    pub fn disconnect_lastfm(&mut self, cx: &mut Context<Self>) {
        self.pending_lastfm_token = None;
        self.settings.update(cx, |settings, cx| {
            settings.set_lastfm_session_key(String::new(), cx);
            settings.set_lastfm_username(String::new(), cx);
        });
    }

    pub fn connect_librefm(&mut self, api_key: String, api_secret: String, cx: &mut Context<Self>) {
        if self.librefm_task.is_some() {
            return;
        }
        self.settings.update(cx, |settings, cx| {
            settings.set_librefm_api_key(api_key.clone(), cx);
            settings.set_librefm_api_secret(api_secret.clone(), cx);
        });
        let io = self.io.clone();
        self.librefm_task = Some(cx.spawn(async move |this, cx| {
            let requested = join(io.spawn(async move {
                music::audioscrobbler::request_token(&LIBREFM, &api_key, &api_secret).await
            }))
            .await;
            this.update(cx, |this, cx| {
                this.librefm_task = None;
                match requested {
                    Ok(token) => {
                        let api_key = this.settings.read(cx).librefm_api_key();
                        let url = music::audioscrobbler::auth_url(&LIBREFM, api_key, &token);
                        this.pending_librefm_token = Some(token);
                        cx.open_url(&url);
                    }
                    Err(error) => {
                        log::warn!("scrobble: cannot request a libre.fm token: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-librefm-failed", cx);
                    }
                }
            })
            .ok();
        }));
    }

    pub fn confirm_librefm(&mut self, cx: &mut Context<Self>) {
        if self.librefm_task.is_some() {
            return;
        }
        let Some(token) = self.pending_librefm_token.clone() else {
            return;
        };
        let (api_key, api_secret) = {
            let settings = self.settings.read(cx);
            (
                settings.librefm_api_key().to_owned(),
                settings.librefm_api_secret().to_owned(),
            )
        };
        let io = self.io.clone();
        self.librefm_task = Some(cx.spawn(async move |this, cx| {
            let exchanged = join(io.spawn(async move {
                music::audioscrobbler::exchange_session(&LIBREFM, &api_key, &api_secret, &token)
                    .await
            }))
            .await;
            this.update(cx, |this, cx| {
                this.librefm_task = None;
                this.pending_librefm_token = None;
                match exchanged {
                    Ok((session_key, username)) => {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_librefm_session_key(session_key, cx);
                            settings.set_librefm_username(username.clone(), cx);
                        });
                        Toasts::about(Outcome::Done, "toast-librefm-connected", username, cx);
                    }
                    Err(error) => {
                        log::warn!("scrobble: cannot complete libre.fm sign-in: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-librefm-failed", cx);
                    }
                }
            })
            .ok();
        }));
    }

    pub fn disconnect_librefm(&mut self, cx: &mut Context<Self>) {
        self.pending_librefm_token = None;
        self.settings.update(cx, |settings, cx| {
            settings.set_librefm_session_key(String::new(), cx);
            settings.set_librefm_username(String::new(), cx);
        });
    }

    pub fn set_listenbrainz_token(&mut self, token: String, cx: &mut Context<Self>) {
        if self.listenbrainz_task.is_some() {
            return;
        }
        let checked = token.clone();
        let io = self.io.clone();
        self.listenbrainz_task = Some(cx.spawn(async move |this, cx| {
            let validated =
                join(io.spawn(async move { music::listenbrainz::validate_token(&checked).await }))
                    .await;
            this.update(cx, |this, cx| {
                this.listenbrainz_task = None;
                match validated {
                    Ok(true) => {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_listenbrainz_token(token, cx)
                        });
                        Toasts::show(Outcome::Done, "toast-listenbrainz-connected", cx);
                    }
                    Ok(false) => {
                        log::warn!("scrobble: listenbrainz rejected the token");
                        Toasts::show(Outcome::Failed, "toast-listenbrainz-failed", cx);
                    }
                    Err(error) => {
                        log::warn!("scrobble: cannot validate a listenbrainz token: {error:#}");
                        Toasts::show(Outcome::Failed, "toast-listenbrainz-failed", cx);
                    }
                }
            })
            .ok();
        }));
    }

    pub fn disconnect_listenbrainz(&mut self, cx: &mut Context<Self>) {
        self.settings.update(cx, |settings, cx| {
            settings.set_listenbrainz_token(String::new(), cx)
        });
    }
}
