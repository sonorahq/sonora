//! Scrobbling
//!
//! One model drives every service at once. [`Scrobbling`] holds a [`ScrobbleRow`] per service, follows
//! [`Playback`] and fans the same listen out to whichever rows are linked and turned on. The
//! provider the track came from hears about it too, so a server such as Navidrome keeps its own
//! play counts and now-playing list.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use music::scrobble::{self, Account, Link, Play, Secret, Service};
use music::{MusicApi, Report, Track};
use tokio::sync::mpsc;

use crate::playback::PlaybackEvent;
use crate::{
    AppSettings, Change, Cx, Event, Io, Job, Playback, PlaybackState, Session, Store, Topic,
};

/// What the settings row says when a link did not go through. The failure itself is in the log.
const FAILED: &str = "settings-scrobble-failed";

/// One state report on its way to the provider that served the track.
type Reported = (Arc<dyn MusicApi>, String, Report, Duration);

/// One service's link as the settings screen sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScrobbleState {
    Off,
    Linking,
    On(String),
    Failed(&'static str),
}

/// A service, its link and the sender that link produced.
pub struct ScrobbleRow {
    service: Arc<dyn Service>,
    /// The stored account as settings last had it, blank when the service was never linked.
    account: Account,
    state: ScrobbleState,
    /// Whether the current play has already gone to this service. Per row, so one service being
    /// down never costs another its listen.
    sent: bool,
}

impl ScrobbleRow {
    pub fn id(&self) -> &'static str {
        self.service.id()
    }

    pub fn link(&self) -> Link {
        self.service.link()
    }

    pub fn signup(&self) -> Option<&'static str> {
        self.service.signup()
    }

    pub fn state(&self) -> &ScrobbleState {
        &self.state
    }

    pub fn linked(&self) -> bool {
        matches!(self.state, ScrobbleState::On(_))
    }

    pub fn linking(&self) -> bool {
        matches!(self.state, ScrobbleState::Linking)
    }

    /// Whether listens are being submitted. A linked service the user switched off stays linked.
    pub fn enabled(&self) -> bool {
        self.account.enabled
    }

    /// Whether listens go to this service right now.
    fn live(&self) -> bool {
        self.account.linked() && self.account.enabled
    }
}

/// Every scrobbling service at once, and the provider the current track came from, fed from
/// [`Playback`]'s starts, ends, pauses and seeks.
pub struct Scrobbling {
    rows: Vec<ScrobbleRow>,
    play: Option<Play>,
    current: Option<String>,
    started: SystemTime,
    /// Whether the current play has already gone to the provider it came from.
    reported: bool,
    /// Where the current track was last heard, for the report that says it stopped.
    position: Duration,
    /// The queue that sends state reports one at a time. A segue puts a stop and the next start
    /// back to back, and the start has to land last or the server forgets the new track.
    reports: mpsc::UnboundedSender<Reported>,
    /// The running link. It owns the service's callback listener, so dropping it frees the port.
    link: Option<Job>,
}

impl Scrobbling {
    pub fn new(settings: &AppSettings, cx: &mut Cx) -> Self {
        let rows = scrobble::services()
            .into_iter()
            .map(|service| ScrobbleRow {
                service,
                account: Account::default(),
                state: ScrobbleState::Off,
                sent: false,
            })
            .collect();

        let mut scrobbling = Self {
            rows,
            play: None,
            current: None,
            started: SystemTime::now(),
            reported: false,
            position: Duration::ZERO,
            reports: send_reports(&cx.io),
            link: None,
        };
        scrobbling.rebuild(settings);
        scrobbling
    }

    pub fn rows(&self) -> &[ScrobbleRow] {
        &self.rows
    }

    /// Drops the running link and its task. The task owns the callback listener, so this frees
    /// the port too.
    fn stop(&mut self) {
        self.link = None;
    }

    /// Rereads every stored account and rebuilds the senders. A row in the middle of a link keeps
    /// its state, so turning another service off cannot wipe the one the user is waiting on.
    fn rebuild(&mut self, settings: &AppSettings) {
        let accounts = settings.scrobbling();

        for row in &mut self.rows {
            row.account = accounts.get(row.id()).cloned().unwrap_or_default();
            if row.linking() {
                continue;
            }
            row.state = match row.account.linked() {
                true => ScrobbleState::On(row.account.name.clone()),
                false => ScrobbleState::Off,
            };
        }
    }

    /// Starts a play and reports it to every live service. A pause and resume of the same track
    /// keeps the original start time, so the listen is timed from when it really began.
    fn begin(&mut self, playback: &Playback, session: &Session, cx: &mut Cx) {
        let Some(track) = playback.track() else {
            return;
        };
        let resumed = track.id.is_some() && self.current == track.id;
        if !resumed {
            self.current = track.id.clone();
            self.started = SystemTime::now();
            self.reported = false;
            for row in &mut self.rows {
                row.sent = false;
            }
        }
        self.play = played(track, self.started);
        self.position = playback.position();
        self.report(Report::Playing, session);
        let Some(play) = self.play.clone() else {
            return;
        };

        for row in &self.rows {
            if !row.live() {
                continue;
            }
            let (service, account, play) = (row.service.clone(), row.account.clone(), play.clone());
            cx.io.spawn(async move {
                if let Err(error) = service.now_playing(&account, &play).await {
                    log::warn!(
                        "scrobble: cannot report the current track to {}: {error:#}",
                        service.id()
                    );
                }
            });
        }
    }

    /// Reports a pause or a landed seek to the provider, for the track `begin` saw start. A
    /// track restored paused at launch was never reported, so it stays quiet until it plays.
    fn moved(&mut self, playback: &Playback, session: &Session) {
        let id = playback.track().and_then(|track| track.id.as_ref());
        if self.current.is_none() || id != self.current.as_ref() {
            return;
        }
        let report = match playback.state() {
            PlaybackState::Playing => Report::Playing,
            _ => Report::Paused,
        };
        self.position = playback.position();
        self.report(report, session);
    }

    /// Tells the provider the track stopped where it was last heard, since `Playback` has
    /// already reset its position by the time the end arrives.
    fn end(&mut self, session: &Session) {
        self.report(Report::Stopped, session);
        self.play = None;
        self.current = None;
        self.reported = false;
        for row in &mut self.rows {
            row.sent = false;
        }
    }

    /// Submits the play to every service that has not had it yet, once it has run long enough.
    fn tick(&mut self, playback: &Playback, session: &Session, cx: &mut Cx) {
        let Some(play) = self.play.as_ref() else {
            return;
        };
        if playback.track().and_then(|track| track.id.as_ref()) == self.current.as_ref() {
            self.position = playback.position();
        }
        if *playback.state() != PlaybackState::Playing || !play.earned(playback.position()) {
            return;
        }

        if !self.reported
            && let Some((client, id)) = self.provider(session)
        {
            self.reported = true;
            let at = play.at;
            cx.io.spawn(async move {
                if let Err(error) = client.played(&id, at).await {
                    log::warn!("scrobble: cannot submit the track to its provider: {error:#}");
                }
            });
        }

        for row in &mut self.rows {
            if row.sent || !row.live() {
                continue;
            }
            row.sent = true;

            let (service, account, play) = (row.service.clone(), row.account.clone(), play.clone());
            cx.io.spawn(async move {
                if let Err(error) = service
                    .scrobble(&account, std::slice::from_ref(&play))
                    .await
                {
                    log::warn!(
                        "scrobble: cannot submit the track to {}: {error:#}",
                        service.id()
                    );
                }
            });
        }
    }

    /// Sends the provider the current track's state at the last position heard.
    fn report(&self, report: Report, session: &Session) {
        let Some((client, id)) = self.provider(session) else {
            return;
        };
        self.reports.send((client, id, report, self.position)).ok();
    }

    /// The client that serves the current track, and the track's id on it. Local files go to
    /// the local client, which keeps no record and ignores the report.
    fn provider(&self, session: &Session) -> Option<(Arc<dyn MusicApi>, String)> {
        let id = self.current.clone()?;
        let client = match music::is_local_id(&id) {
            true => session.local_client(),
            false => session.client(),
        }?;
        Some((client, id))
    }
}

impl Store {
    /// Turns submissions to one service on or off. The link survives either way.
    pub fn set_scrobble_enabled(&mut self, service: &str, enabled: bool) {
        self.settings.set_scrobbling(service, enabled, &mut self.cx);
        self.scrobbling.rebuild(&self.settings);
    }

    /// Runs one service's link. A `Link::Browser` or `Link::Keys` service opens the browser from
    /// inside the task, so the row sits in `Linking` until the user comes back or the wait ends.
    pub fn connect_scrobbler(&mut self, service: &str, secret: Secret) {
        let scrobbling = &mut self.scrobbling;
        let Some(index) = scrobbling.rows.iter().position(|row| row.id() == service) else {
            return;
        };

        scrobbling.stop();
        let service = scrobbling.rows[index].service.clone();
        scrobbling.rows[index].state = ScrobbleState::Linking;

        scrobbling.link = Some(self.cx.spawn(move |link| async move {
            let linked = service.connect(secret).await;

            link.post(move |store| {
                let Store {
                    scrobbling,
                    settings,
                    cx,
                    ..
                } = store;
                scrobbling.link = None;
                match linked {
                    Ok(account) => {
                        let id = scrobbling.rows[index].id();
                        scrobbling.rows[index].state = ScrobbleState::Off;
                        settings.set_account(id, account, cx);
                        scrobbling.rebuild(settings);
                    }
                    Err(error) => {
                        log::warn!(
                            "scrobble: cannot link {}: {error:#}",
                            scrobbling.rows[index].id()
                        );
                        scrobbling.rows[index].state = ScrobbleState::Failed(FAILED);
                    }
                }
            });
        }));
    }

    /// Forgets one service's account. A link still running is dropped first.
    pub fn disconnect_scrobbler(&mut self, service: &str) {
        self.scrobbling.stop();
        self.settings
            .set_account(service, Account::default(), &mut self.cx);
        self.scrobbling.rebuild(&self.settings);
    }

    /// Gives up on a link that is still waiting, so the user is not stuck watching a browser tab
    /// they already closed. The row falls back to whatever is stored for it.
    pub fn cancel_scrobble_link(&mut self) {
        let scrobbling = &mut self.scrobbling;
        scrobbling.stop();
        for row in &mut scrobbling.rows {
            if row.linking() {
                row.state = ScrobbleState::Off;
            }
        }
        scrobbling.rebuild(&self.settings);
    }

    pub(crate) fn react_scrobbling(&mut self, change: &Change) {
        let Store {
            scrobbling,
            playback,
            session,
            cx,
            ..
        } = self;
        match change {
            Change::Event(Event::Playback(event)) => match event {
                PlaybackEvent::StartedPlayback => scrobbling.begin(playback, session, cx),
                PlaybackEvent::EndedPlayback => scrobbling.end(session),
                PlaybackEvent::Paused | PlaybackEvent::Seeked => {
                    scrobbling.moved(playback, session)
                }
            },
            Change::Topic(Topic::Playback) => scrobbling.tick(playback, session, cx),
            _ => {}
        }
    }
}

/// Turns a track into a listen. A track with no artist or no title is not submittable anywhere.
fn played(track: &Track, at: SystemTime) -> Option<Play> {
    let artist = track
        .artist_refs
        .first()
        .map(|artist| artist.name.clone())
        .unwrap_or_else(|| track.artists.clone());
    if artist.is_empty() || track.name.is_empty() {
        return None;
    }

    Some(Play {
        artist,
        title: track.name.clone(),
        album: Some(track.album.clone()).filter(|album| !album.is_empty()),
        duration: track.duration,
        at,
    })
}

/// Starts the task that sends state reports in the order they were queued. It ends when the
/// model drops its sender.
fn send_reports(io: &Io) -> mpsc::UnboundedSender<Reported> {
    let (sender, mut queued) = mpsc::unbounded_channel::<Reported>();
    io.spawn(async move {
        while let Some((client, id, report, position)) = queued.recv().await {
            if let Err(error) = client.report(&id, report, position).await {
                log::warn!("scrobble: cannot report the track to its provider: {error:#}");
            }
        }
    });
    sender
}
