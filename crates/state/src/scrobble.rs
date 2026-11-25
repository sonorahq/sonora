//! Scrobbling
//!
//! One entity drives every service at once. [`Scrobbling`] holds a [`ScrobbleRow`] per service, follows
//! [`Playback`] and fans the same listen out to whichever rows are linked and turned on. The
//! provider the track came from hears about it too, so a server such as Navidrome keeps its own
//! play counts and now-playing list.

use std::time::{Duration, SystemTime};

use gpui::{App, Context, Entity, SharedString, Task};
use music::scrobble::{self, Account, KeyRefused, Link, Play, Secret, Service};
use music::{MusicApi, Report, Track};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::playback::PlaybackEvent;
use crate::{AppSettings, Io, Outcome, Playback, PlaybackState, Session, Toast, Toasts, join};

/// What the settings row says when a link did not go through. The failure itself is in the log.
const FAILED: &str = "settings-scrobble-failed";
/// What the settings row says when the service refused the API key while linking.
const KEY_REFUSED: &str = "settings-scrobble-key-refused";

/// One state report on its way to the provider that served the track.
type Reported = (Arc<dyn MusicApi>, String, Report, Duration);
/// Whether one answer from a service refused the API key. It carries the session it was sent
/// with, so an answer that arrives after the account was replaced is dropped.
struct Verdict {
    service: &'static str,
    session: String,
    refused: bool,
}

/// One service's link as the settings screen sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScrobbleState {
    Off,
    Linking,
    On(SharedString),
    /// Linked, but the service refuses the stored API key, so no listen gets through until the
    /// key's owner fixes it upstream. Last.fm does this to keys whose owner has no Pro.
    Refused,
    Failed(&'static str),
}

/// A service, its link and the sender that link produced.
pub struct ScrobbleRow {
    service: Arc<dyn Service>,
    /// The account stored in the accounts folder, blank when the service was never linked.
    account: Account,
    /// Whether settings leave this service switched on.
    enabled: bool,
    state: ScrobbleState,
    /// Whether the current play has already gone to this service. Per row, so one service being
    /// down never costs another its listen.
    sent: bool,
    /// Whether the service refused the stored key on its last answer. Submissions go on, so the
    /// row recovers by itself once the key is accepted again.
    refused: bool,
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
        matches!(self.state, ScrobbleState::On(_) | ScrobbleState::Refused)
    }

    pub fn linking(&self) -> bool {
        matches!(self.state, ScrobbleState::Linking)
    }

    /// Whether listens are being submitted. A linked service the user switched off stays linked.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The stored account, which the link dialog fills its fields from.
    pub fn account(&self) -> &Account {
        &self.account
    }

    /// Whether listens go to this service right now.
    fn live(&self) -> bool {
        self.account.linked() && self.enabled
    }
}

pub struct Scrobbling {
    playback: Entity<Playback>,
    session: Entity<Session>,
    settings: Entity<AppSettings>,
    io: Io,
    rows: Vec<ScrobbleRow>,
    play: Option<Play>,
    current: Option<String>,
    started: SystemTime,
    /// How many plays have begun since launch, which numbers each one as its [`Play::order`].
    plays: u64,
    /// Whether the current play has already gone to the provider it came from.
    reported: bool,
    /// Where the current track was last heard, for the report that says it stopped.
    position: Duration,
    /// The queue that sends state reports one at a time. A segue puts a stop and the next start
    /// back to back, and the start has to land last or the server forgets the new track.
    reports: mpsc::UnboundedSender<Reported>,
    /// Where submissions say whether the service took the key, which `_hearing` applies.
    verdicts: mpsc::UnboundedSender<Verdict>,
    _hearing: Task<()>,
    link: Option<Task<()>>,
    waiting: Option<AbortHandle>,
}

impl Scrobbling {
    pub fn new(
        playback: Entity<Playback>,
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&playback, |this, _, event, cx| match event {
            PlaybackEvent::StartedPlayback => this.begin(cx),
            PlaybackEvent::EndedPlayback => this.end(cx),
            PlaybackEvent::Paused | PlaybackEvent::Seeked => this.moved(cx),
        })
        .detach();
        cx.observe(&playback, |this, _, cx| this.tick(cx)).detach();
        // a play is earned by the time listened, which moves on position reports
        let ticks = playback.read(cx).ticks().clone();
        cx.observe(&ticks, |this, _, cx| this.tick(cx)).detach();

        let rows = scrobble::services()
            .into_iter()
            .map(|service| ScrobbleRow {
                service,
                account: Account::default(),
                enabled: true,
                state: ScrobbleState::Off,
                sent: false,
                refused: false,
            })
            .collect();

        let (verdicts, mut heard) = mpsc::unbounded_channel::<Verdict>();
        let hearing = cx.spawn(async move |this, cx| {
            while let Some(verdict) = heard.recv().await {
                let applied = this.update(cx, |this, cx| this.judged(verdict, cx));
                if applied.is_err() {
                    break;
                }
            }
        });

        let mut scrobbling = Self {
            playback,
            session,
            settings,
            io: io.clone(),
            rows,
            play: None,
            current: None,
            started: SystemTime::now(),
            plays: 0,
            reported: false,
            position: Duration::ZERO,
            reports: send_reports(&io),
            verdicts,
            _hearing: hearing,
            link: None,
            waiting: None,
        };
        scrobbling.load(cx);
        scrobbling.rebuild(cx);
        scrobbling
    }

    pub fn rows(&self) -> &[ScrobbleRow] {
        &self.rows
    }

    /// One service's stored account, blank when it was never linked.
    pub fn account(&self, service: &str) -> Account {
        self.rows
            .iter()
            .find(|row| row.id() == service)
            .map(|row| row.account.clone())
            .unwrap_or_default()
    }

    /// Turns submissions to one service on or off. The link survives either way.
    pub fn set_enabled(&mut self, service: &str, enabled: bool, cx: &mut Context<Self>) {
        self.settings.update(cx, |settings, cx| {
            settings.set_scrobbling(service, enabled, cx)
        });
        self.rebuild(cx);
    }

    /// Runs one service's link. A `Link::Browser` or `Link::Keys` service opens the browser from
    /// inside the task, so the row sits in `Linking` until the user comes back or the wait ends.
    pub fn connect(&mut self, service: &str, secret: Secret, cx: &mut Context<Self>) {
        let Some(index) = self.rows.iter().position(|row| row.id() == service) else {
            return;
        };

        self.stop();
        let service = self.rows[index].service.clone();
        self.rows[index].state = ScrobbleState::Linking;
        cx.notify();

        let connecting = self.io.spawn(async move {
            let account = service.connect(secret).await?;
            scrobble::store(service.id(), &account)?;
            anyhow::Ok(account)
        });
        self.waiting = Some(connecting.abort_handle());

        self.link = Some(cx.spawn(async move |this, cx| {
            let linked = join(connecting).await;

            this.update(cx, |this, cx| {
                this.link = None;
                this.waiting = None;
                match linked {
                    Ok(account) => {
                        let id = this.rows[index].id();
                        this.rows[index].account = account;
                        this.rows[index].state = ScrobbleState::Off;
                        this.rows[index].refused = false;
                        this.settings
                            .update(cx, |settings, cx| settings.set_scrobbling(id, true, cx));
                        this.rebuild(cx);
                    }
                    Err(error) => {
                        log::warn!("scrobble: cannot link {}: {error:#}", this.rows[index].id());
                        this.rows[index].state =
                            ScrobbleState::Failed(match KeyRefused::in_error(&error) {
                                true => KEY_REFUSED,
                                false => FAILED,
                            });
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Forgets one service's account. A link still running is dropped first.
    pub fn disconnect(&mut self, service: &str, cx: &mut Context<Self>) {
        self.stop();
        scrobble::forget(service);
        if let Some(row) = self.rows.iter_mut().find(|row| row.id() == service) {
            row.account = Account::default();
            row.refused = false;
        }
        self.settings.update(cx, |settings, cx| {
            settings.set_scrobbling(service, true, cx)
        });
        self.rebuild(cx);
    }

    /// Gives up on a link that is still waiting, so the user is not stuck watching a browser tab
    /// they already closed. The row falls back to whatever is stored for it.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.stop();
        for row in &mut self.rows {
            if row.linking() {
                row.state = ScrobbleState::Off;
            }
        }
        self.rebuild(cx);
    }

    /// Drops the running link and its task. The task owns the callback listener, so this frees
    /// the port too.
    fn stop(&mut self) {
        self.link = None;
        if let Some(waiting) = self.waiting.take() {
            waiting.abort();
        }
    }

    /// Reads every service's account from the accounts folder, first moving the ones an older
    /// Sonora left in `settings.json` there. An account that cannot move stays in settings for
    /// the next launch to try again, and its service reads as unlinked until then.
    // TODO(0.52.0): drop the move out of settings.json and only read the accounts.
    fn load(&mut self, cx: &mut Context<Self>) {
        let legacy = self.settings.read(cx).legacy_scrobbling().clone();
        let mut moved = Vec::new();
        for (service, old) in legacy {
            match scrobble::store(&service, &old.account) {
                Ok(()) => {
                    log::info!("scrobble: moved the {service} account out of settings.json");
                    moved.push((service, old.enabled));
                }
                Err(error) => log::warn!(
                    "scrobble: cannot move the {service} account out of settings.json: {error:#}"
                ),
            }
        }
        self.settings.update(cx, |settings, cx| {
            settings.forget_legacy_scrobbling(&moved, cx)
        });

        for row in &mut self.rows {
            row.account = scrobble::load(row.id());
        }
    }

    /// Rereads which services are switched on and rebuilds each row's state. A row in the middle
    /// of a link keeps its state, so turning another service off cannot wipe the one the user is
    /// waiting on.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.read(cx);

        for row in &mut self.rows {
            row.enabled = settings.scrobbling_on(row.id());
            if row.linking() {
                continue;
            }
            row.state = match (row.account.linked(), row.refused) {
                (true, true) => ScrobbleState::Refused,
                (true, false) => ScrobbleState::On(SharedString::from(row.account.name.clone())),
                (false, _) => ScrobbleState::Off,
            };
        }
        cx.notify();
    }

    /// Applies what a service said about the stored key. The first refusal in a run of them
    /// posts a toast, since a listener rarely looks at the settings row while music plays.
    fn judged(&mut self, verdict: Verdict, cx: &mut Context<Self>) {
        let Verdict {
            service,
            session,
            refused,
        } = verdict;
        let Some(row) = self.rows.iter_mut().find(|row| row.id() == service) else {
            return;
        };
        if row.refused == refused || !row.account.linked() || row.account.session != session {
            return;
        }
        row.refused = refused;
        if refused {
            log::warn!("scrobble: {service} refuses the stored api key");
            let name = i18n::lookup(&format!("settings-{service}"), None);
            Toasts::post(
                Toast::new(Outcome::Warning, "toast-scrobble-key-refused")
                    .name(name)
                    .note("toast-scrobble-key-refused-note"),
                cx,
            );
        }
        self.rebuild(cx);
    }

    /// Starts a play and reports it to every live service. A pause and resume of the same track
    /// keeps the original start time, so the listen is timed from when it really began.
    fn begin(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        let Some(track) = playback.track() else {
            return;
        };
        let resumed = track.id.is_some() && self.current == track.id;
        if !resumed {
            self.current = track.id.clone();
            self.started = SystemTime::now();
            self.plays += 1;
            self.reported = false;
            for row in &mut self.rows {
                row.sent = false;
            }
        }
        self.play = played(track, self.started, self.plays);
        self.position = playback.position();
        self.report(Report::Playing, cx);
        let Some(play) = self.play.clone() else {
            return;
        };

        let muted = self.muted(cx);
        for row in &self.rows {
            if muted || !row.live() {
                continue;
            }
            let (service, account, play) = (row.service.clone(), row.account.clone(), play.clone());
            let verdicts = self.verdicts.clone();
            self.io.spawn(async move {
                let answer = service.now_playing(&account, &play).await;
                if let Err(error) = &answer {
                    log::warn!(
                        "scrobble: cannot report the current track to {}: {error:#}",
                        service.id()
                    );
                }
                judge(&verdicts, service.id(), &account, &answer);
            });
        }
    }

    /// Reports a pause or a landed seek to the provider, for the track `begin` saw start. A
    /// track restored paused at launch was never reported, so it stays quiet until it plays.
    fn moved(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        let id = playback.track().and_then(|track| track.id.as_ref());
        if self.current.is_none() || id != self.current.as_ref() {
            return;
        }
        let report = match playback.state() {
            PlaybackState::Playing => Report::Playing,
            _ => Report::Paused,
        };
        self.position = playback.position();
        self.report(report, cx);
    }

    /// Tells the provider the track stopped where it was last heard, since `Playback` has
    /// already reset its position by the time the end arrives.
    fn end(&mut self, cx: &mut Context<Self>) {
        self.report(Report::Stopped, cx);
        self.play = None;
        self.current = None;
        self.reported = false;
        for row in &mut self.rows {
            row.sent = false;
        }
    }

    /// Submits the play to every service that has not had it yet, once it has run long enough.
    fn tick(&mut self, cx: &mut Context<Self>) {
        let Some(play) = self.play.as_ref() else {
            return;
        };
        let playback = self.playback.read(cx);
        if playback.track().and_then(|track| track.id.as_ref()) == self.current.as_ref() {
            self.position = playback.position();
        }
        let played = playback.position();
        if *playback.state() != PlaybackState::Playing || !play.earned(played) {
            return;
        }

        if !self.reported
            && let Some((client, id)) = self.provider(cx)
        {
            self.reported = true;
            let at = play.at;
            self.io.spawn(async move {
                if let Err(error) = client.played(&id, at).await {
                    log::warn!("scrobble: cannot submit the track to its provider: {error:#}");
                }
            });
        }

        let muted = self.muted(cx);
        for row in &mut self.rows {
            if muted || row.sent || !row.live() || !row.service.earned(play, played) {
                continue;
            }
            row.sent = true;

            let (service, account, play) = (row.service.clone(), row.account.clone(), play.clone());
            let verdicts = self.verdicts.clone();
            self.io.spawn(async move {
                let answer = service
                    .scrobble(&account, std::slice::from_ref(&play))
                    .await;
                if let Err(error) = &answer {
                    log::warn!(
                        "scrobble: cannot submit the track to {}: {error:#}",
                        service.id()
                    );
                }
                judge(&verdicts, service.id(), &account, &answer);
            });
        }
    }

    /// Whether the current track came from the active provider and the user asked to keep that
    /// provider's plays out of the linked services, usually because it scrobbles them on its own.
    /// Local files always go through, and the provider itself still hears the play.
    fn muted(&self, cx: &App) -> bool {
        let Some(id) = self.current.as_deref() else {
            return false;
        };
        let Some(provider) = self.session.read(cx).provider_slug() else {
            return false;
        };
        !music::is_local_id(id) && !self.settings.read(cx).scrobbles(provider)
    }

    /// Sends the provider the current track's state at the last position heard.
    fn report(&self, report: Report, cx: &App) {
        let Some((client, id)) = self.provider(cx) else {
            return;
        };
        self.reports.send((client, id, report, self.position)).ok();
    }

    /// The client that serves the current track, and the track's id on it. Local files go to
    /// the local client, which keeps no record and ignores the report.
    fn provider(&self, cx: &App) -> Option<(Arc<dyn MusicApi>, String)> {
        let id = self.current.clone()?;
        let session = self.session.read(cx);
        let client = match music::is_local_id(&id) {
            true => session.local_client(),
            false => session.client(),
        }?;
        Some((client, id))
    }
}

/// Tells the entity whether a service took the key, from the answer to one submission. Any other
/// failure says nothing about the key, so it sends nothing.
fn judge(
    verdicts: &mpsc::UnboundedSender<Verdict>,
    service: &'static str,
    account: &Account,
    answer: &anyhow::Result<()>,
) {
    let refused = match answer {
        Ok(()) => false,
        Err(error) if KeyRefused::in_error(error) => true,
        Err(_) => return,
    };
    let session = account.session.clone();
    verdicts
        .send(Verdict {
            service,
            session,
            refused,
        })
        .ok();
}

/// Turns a track into a listen. A track with no artist or no title is not submittable anywhere.
fn played(track: &Track, at: SystemTime, order: u64) -> Option<Play> {
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
        order,
    })
}

/// Starts the task that sends state reports in the order they were queued. It ends when the
/// entity drops its sender.
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
