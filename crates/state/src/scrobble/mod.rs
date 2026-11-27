//! Scrobbling
//!
//! One entity drives every service at once. [`Scrobbling`] holds a [`ScrobbleRow`] per service, follows
//! [`Playback`] and fans the same listen out to whichever rows are linked and turned on. The
//! provider the track came from hears about it too, so a server such as Navidrome keeps its own
//! play counts and now-playing list. A listen waits in a backlog until its service takes it, so
//! one made offline is sent once the service can be reached.

use std::time::{Duration, Instant, SystemTime};

use futures::future::join_all;
use gpui::{App, Context, Entity, SharedString, Task};
use music::{MusicApi, Report, Track};
use scrobble::{self, Account, KeyRefused, Link, Play, Secret, Service};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use storage::Database;

use crate::playback::PlaybackEvent;
use crate::{
    AppSettings, Io, Network, Outcome, Playback, PlaybackState, Reconnected, Session, Toast,
    Toasts, join,
};
use backlog::{Backlog, Submit};

mod backlog;

/// What the settings row says when a link did not go through. The failure itself is in the log.
const FAILED: &str = "settings-scrobble-failed";
/// What the settings row says when the service refused the API key while linking.
const KEY_REFUSED: &str = "settings-scrobble-key-refused";

/// One state report on its way to the provider that served the track.
type Reported = (Arc<dyn MusicApi>, String, Report, Duration);
/// One change of what a service shows as playing, on its way to that service's queue.
type Told = (Account, Now);

/// What a service should show as playing right now.
enum Now {
    /// This play, at this position.
    Playing(Play, Duration),
    /// Nothing, after a pause or the end of playback.
    Stopped,
}

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
    /// The queue that tells this service what plays. See [`tell`].
    now: mpsc::UnboundedSender<Told>,
    /// Whether the service was last told that something plays, so a pause, an end or a quit
    /// has a status to take back.
    shown: bool,
    /// Keeps this service's listens and sends them.
    submit: backlog::Sender,
    /// The account the sender was last handed, `None` while it holds the listens back.
    handed: Option<Account>,
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

    /// Queues what the service should show as playing.
    fn tell(&mut self, now: Now) {
        self.shown = matches!(now, Now::Playing(..));
        self.now.send((self.account.clone(), now)).ok();
    }

    /// Takes back a status this service still shows.
    fn hide(&mut self) {
        if self.shown {
            self.tell(Now::Stopped);
        }
    }

    /// Hands the sender the account while the service is live, or holds the listens back while
    /// it is not. The sender hears only a change, unless `again` asks it to try what waits now.
    fn hand(&mut self, again: bool) {
        let account = self.live().then(|| self.account.clone());
        if !again && account == self.handed {
            return;
        }
        self.handed = account.clone();
        let submit = match account {
            Some(account) => Submit::Account(account),
            None => Submit::Hold,
        };
        self.submit.ask(submit);
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
    /// How long the current play has played in the stretches that already ended. Seeks add
    /// nothing, so skipping ahead never earns a listen.
    listened: Duration,
    /// When the stretch playing right now began, `None` while paused or loading.
    since: Option<Instant>,
    /// Whether the current play has already gone to the provider it came from.
    reported: bool,
    /// Where the current track was last heard, for the report that says it stopped.
    position: Duration,
    /// The queue that sends state reports one at a time. A segue puts a stop and the next start
    /// back to back, and the start has to land last or the server forgets the new track.
    reports: mpsc::UnboundedSender<Reported>,
    /// Applies what the services' answers say about the stored key. The row tasks hold the
    /// senders, so it runs as long as they do.
    _hearing: Task<()>,
    link: Option<Task<()>>,
    waiting: Option<AbortHandle>,
}

impl Scrobbling {
    pub fn new(
        playback: Entity<Playback>,
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        network: &Entity<Network>,
        database: Database,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        // the network coming back is the likeliest moment for waiting listens to get through
        cx.subscribe(network, |this, _, _: &Reconnected, _| {
            this.rows.iter_mut().for_each(|row| row.hand(true))
        })
        .detach();
        cx.subscribe(&playback, |this, _, event, cx| match event {
            PlaybackEvent::StartedPlayback => this.begin(cx),
            PlaybackEvent::EndedPlayback => this.end(cx),
            PlaybackEvent::Paused | PlaybackEvent::Seeked => this.moved(cx),
        })
        .detach();
        cx.observe(&playback, |this, _, cx| this.tick(cx)).detach();
        // whether a play has earned its listen is checked on every position report
        let ticks = playback.read(cx).ticks().clone();
        cx.observe(&ticks, |this, _, cx| this.tick(cx)).detach();
        cx.on_app_quit(|this, _| this.quit()).detach();

        let (verdicts, mut heard) = mpsc::unbounded_channel::<Verdict>();
        let hearing = cx.spawn(async move |this, cx| {
            while let Some(verdict) = heard.recv().await {
                let applied = this.update(cx, |this, cx| this.judged(verdict, cx));
                if applied.is_err() {
                    break;
                }
            }
        });

        let backlog = Backlog::new(database);
        let rows = scrobble::services()
            .into_iter()
            .map(|service| ScrobbleRow {
                now: tell(&io, service.clone(), verdicts.clone()),
                submit: backlog::Sender::new(
                    &io,
                    service.clone(),
                    backlog.clone(),
                    verdicts.clone(),
                ),
                handed: None,
                service,
                account: Account::default(),
                enabled: true,
                state: ScrobbleState::Off,
                sent: false,
                refused: false,
                shown: false,
            })
            .collect();

        let mut scrobbling = Self {
            playback,
            session,
            settings,
            io: io.clone(),
            rows,
            play: None,
            current: None,
            started: SystemTime::now(),
            listened: Duration::ZERO,
            since: None,
            reported: false,
            position: Duration::ZERO,
            reports: send_reports(&io),
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

    /// Turns submissions to one service on or off. The link survives either way, and a service
    /// switched off stops showing the current track.
    pub fn set_enabled(&mut self, service: &str, enabled: bool, cx: &mut Context<Self>) {
        if !enabled && let Some(row) = self.rows.iter_mut().find(|row| row.id() == service) {
            row.hide();
        }
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
                        this.rows[index].hide();
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

    /// Forgets one service's account and the listens still waiting for it, taking back its status
    /// first while the account can still sign the request. A link still running is dropped first.
    pub fn disconnect(&mut self, service: &str, cx: &mut Context<Self>) {
        self.stop();
        scrobble::forget(service);
        if let Some(row) = self.rows.iter_mut().find(|row| row.id() == service) {
            row.hide();
            row.submit.ask(Submit::Forget);
            row.handed = None;
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
            row.hand(false);
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
            self.listened = Duration::ZERO;
            self.since = None;
            self.reported = false;
            for row in &mut self.rows {
                row.sent = false;
            }
        }
        self.play = played(track, self.started);
        self.position = playback.position();
        self.listening(true);
        self.report(Report::Playing, cx);
        self.show(cx);
    }

    /// Tells every live service that the current play is playing from the last position heard.
    /// A service that should not hear it takes back whatever it still shows.
    fn show(&mut self, cx: &App) {
        let muted = self.muted(cx);
        for row in &mut self.rows {
            match (&self.play, muted || !row.live()) {
                (Some(play), false) => row.tell(Now::Playing(play.clone(), self.position)),
                _ => row.hide(),
            }
        }
    }

    /// Reports a pause or a landed seek to the provider, for the track `begin` saw start. A
    /// pause takes the status back from the services, and a seek moves it to the new position.
    /// A track restored paused at launch was never reported, so it stays quiet until it plays.
    fn moved(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        let id = playback.track().and_then(|track| track.id.as_ref());
        if self.current.is_none() || id != self.current.as_ref() {
            return;
        }
        let playing = *playback.state() == PlaybackState::Playing;
        self.position = playback.position();
        self.listening(playing);
        match playing {
            true => {
                self.report(Report::Playing, cx);
                self.show(cx);
            }
            false => {
                self.report(Report::Paused, cx);
                self.rows.iter_mut().for_each(ScrobbleRow::hide);
            }
        }
    }

    /// Tells the provider the track stopped where it was last heard, since `Playback` has
    /// already reset its position by the time the end arrives, and takes the status back from
    /// the services. The next track, if one follows, shows its own a moment later.
    fn end(&mut self, cx: &mut Context<Self>) {
        self.report(Report::Stopped, cx);
        self.play = None;
        self.current = None;
        self.reported = false;
        self.listened = Duration::ZERO;
        self.since = None;
        for row in &mut self.rows {
            row.sent = false;
            row.hide();
        }
    }

    /// Opens or closes the stretch of listening as playback starts or stops moving.
    fn listening(&mut self, playing: bool) {
        match (playing, self.since) {
            (true, None) => self.since = Some(Instant::now()),
            (false, Some(since)) => {
                self.listened += since.elapsed();
                self.since = None;
            }
            _ => {}
        }
    }

    /// How long the current play has actually played, the running stretch included.
    fn time_listened(&self) -> Duration {
        self.listened + self.since.map_or(Duration::ZERO, |since| since.elapsed())
    }

    /// Takes back every status still shown, for the app to wait on as it quits. The wait is
    /// short, so a slow service keeps the track until its status runs out by itself.
    fn quit(&mut self) -> impl Future<Output = ()> + use<> {
        let io = &self.io;
        let stopping: Vec<_> = self
            .rows
            .iter_mut()
            .filter(|row| row.shown)
            .map(|row| {
                row.shown = false;
                let (service, account) = (row.service.clone(), row.account.clone());
                io.spawn(async move {
                    if let Err(error) = service.stopped(&account).await {
                        log::warn!(
                            "scrobble: cannot take the status back from {}: {error:#}",
                            service.id()
                        );
                    }
                })
            })
            .collect();
        async move {
            join_all(stopping).await;
        }
    }

    /// Submits the play to every service that has not had it yet, once it has played long
    /// enough.
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.play.is_none() {
            return;
        }
        let playback = self.playback.read(cx);
        let current = playback.track().and_then(|track| track.id.as_ref()) == self.current.as_ref();
        let playing = current && *playback.state() == PlaybackState::Playing;
        if current {
            self.position = playback.position();
        }
        self.listening(playing);
        let played = self.time_listened();
        let Some(play) = self.play.as_ref() else {
            return;
        };
        if !playing || !play.earned(played) {
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
            row.submit.play(row.account.clone(), play.clone());
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

/// Starts the task that tells one service what plays, one call at a time and in the order they
/// were queued, so a late answer can never bring back a status already taken back. Changes that
/// pile up behind a slow call collapse into the newest. It ends when the row drops its sender.
fn tell(
    io: &Io,
    service: Arc<dyn Service>,
    verdicts: mpsc::UnboundedSender<Verdict>,
) -> mpsc::UnboundedSender<Told> {
    let (sender, mut queued) = mpsc::unbounded_channel::<Told>();
    io.spawn(async move {
        while let Some(mut told) = queued.recv().await {
            while let Ok(newer) = queued.try_recv() {
                told = newer;
            }
            let (account, now) = told;
            let answer = match &now {
                Now::Playing(play, position) => {
                    service.now_playing(&account, play, *position).await
                }
                Now::Stopped => service.stopped(&account).await,
            };
            if let Err(error) = &answer {
                log::warn!(
                    "scrobble: cannot tell {} what is playing: {error:#}",
                    service.id()
                );
            }
            judge(&verdicts, service.id(), &account, &answer);
        }
    });
    sender
}
