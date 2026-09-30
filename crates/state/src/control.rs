use std::time::Duration;

use control::channel::{self, Client, CommandResult, Publisher, Request, SequencedEvent};
use control::{
    AppSnapshot, Command, CommandReply, ControlError, ControlEvent, PlaybackSnapshot,
    PlaybackStatus, QueueSnapshot, RepeatMode, SessionSnapshot, SessionStatus, TrackSummary,
};
use gpui::prelude::*;
use gpui::{App, Context, Entity, Global, Task};
use music::Track;
use tokio::sync::mpsc;

use crate::{Playback, PlaybackState, Queue, Repeat, Session, SessionEvent, SessionState, Sonora};

/// How much history a snapshot carries. The past truncates from the front, so what remains is
/// the most recently played.
const PAST_LIMIT: usize = 50;
/// How much of the upcoming queue a snapshot carries. It truncates from the tail only, so
/// snapshot indices still address the live queue.
const UPCOMING_LIMIT: usize = 200;
/// How many radio suggestions a snapshot carries.
const SUGGESTED_LIMIT: usize = 50;
/// How often playing positions are sampled for event subscribers.
const POSITION_EVERY: Duration = Duration::from_secs(1);

const PLAYBACK: &[Section] = &[Section::Playback];
const QUEUE: &[Section] = &[Section::Queue];
const QUEUE_PLAYBACK: &[Section] = &[Section::Queue, Section::Playback];

struct Attached {
    _host: Entity<ControlHost>,
    client: Client,
}

impl Global for Attached {}

/// Which snapshot sections one change touched. A refresh publishes a single revision with one
/// event per section.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Playback,
    Queue,
    Session,
}

/// Attaches the control bridge to the running app and returns a client for plugins. Safe to
/// call twice: the second call returns the existing client.
pub fn attach_control(cx: &mut App) -> Client {
    if cx.has_global::<Attached>() {
        return cx.global::<Attached>().client.clone();
    }
    let sonora = Sonora::global(cx);
    attach_for(
        sonora.playback.clone(),
        sonora.queue.clone(),
        sonora.session.clone(),
        cx,
    )
}

fn attach_for(
    playback: Entity<Playback>,
    queue: Entity<Queue>,
    session: Entity<Session>,
    cx: &mut App,
) -> Client {
    let initial = snapshot(0, playback.read(cx), queue.read(cx), session.read(cx));
    let (host, client) = channel::pair(initial.clone());
    let (requests, publish) = host.split();
    let entity =
        cx.new(|cx| ControlHost::new(requests, publish, initial, playback, queue, session, cx));
    cx.set_global(Attached {
        _host: entity,
        client: client.clone(),
    });
    client
}

/// The GPUI side of the control API: answers requests from `Client` by driving the existing
/// `Playback`, `Queue` and `Session` entities, and publishes sanitized snapshots and events.
/// Only `attach_control` builds one.
pub struct ControlHost {
    publish: Publisher,
    playback: Entity<Playback>,
    queue: Entity<Queue>,
    session: Entity<Session>,
    snapshot_revision: u64,
    event_seq: u64,
    last: AppSnapshot,
    _requests: Task<()>,
    _position: Task<()>,
}

impl ControlHost {
    fn new(
        requests: mpsc::Receiver<Request>,
        publish: Publisher,
        initial: AppSnapshot,
        playback: Entity<Playback>,
        queue: Entity<Queue>,
        session: Entity<Session>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _requests = cx.spawn(async move |this, cx| {
            let mut requests = requests;
            while let Some(request) = requests.recv().await {
                if this
                    .update(cx, |this, cx| this.answer(request, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let _position = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POSITION_EVERY).await;
                if this
                    .update(cx, |this, cx| this.sample_position(cx))
                    .is_err()
                {
                    break;
                }
            }
        });

        cx.observe(&playback, |this, _, cx| {
            this.refresh(PLAYBACK, cx);
        })
        .detach();
        cx.observe(&queue, |this, _, cx| {
            this.refresh(QUEUE, cx);
        })
        .detach();
        cx.observe(&session, |this, _, cx| {
            this.refresh(&[Section::Session], cx);
        })
        .detach();
        cx.subscribe(&session, |this, _, _: &SessionEvent, cx| {
            this.refresh(&[Section::Session], cx);
        })
        .detach();

        Self {
            publish,
            playback,
            queue,
            session,
            snapshot_revision: 0,
            event_seq: 0,
            last: initial,
            _requests,
            _position,
        }
    }

    fn answer(&mut self, request: Request, cx: &mut Context<Self>) {
        match request {
            Request::GetSnapshot { reply, .. } => {
                let _ = reply.send(self.build(cx, self.snapshot_revision));
            }
            Request::Execute { command, reply, .. } => {
                let _ = reply.send(self.run(&command, cx));
            }
        }
    }

    fn run(&mut self, command: &Command, cx: &mut Context<Self>) -> CommandResult {
        command.validate()?;
        let sections: &[Section] = match command {
            Command::Play => {
                self.playback.update(cx, |playback, cx| playback.resume(cx));
                PLAYBACK
            }
            Command::Pause => {
                self.playback.update(cx, |playback, cx| playback.pause(cx));
                PLAYBACK
            }
            Command::Toggle => {
                self.playback
                    .update(cx, |playback, cx| playback.toggle_play(cx));
                PLAYBACK
            }
            Command::Next => {
                self.playback.update(cx, |playback, cx| playback.next(cx));
                QUEUE_PLAYBACK
            }
            Command::Previous => {
                self.playback
                    .update(cx, |playback, cx| playback.previous(cx));
                QUEUE_PLAYBACK
            }
            Command::Seek { position_ms } => {
                self.playback.update(cx, |playback, cx| {
                    playback.seek(Duration::from_millis(*position_ms), cx)
                });
                PLAYBACK
            }
            Command::SetVolume { level } => {
                self.playback
                    .update(cx, |playback, cx| playback.set_volume(*level, cx));
                PLAYBACK
            }
            Command::SetRepeat { mode } => {
                let repeat = repeat_requested(*mode);
                self.playback
                    .update(cx, |playback, cx| playback.set_repeat(repeat, cx));
                PLAYBACK
            }
            Command::SetShuffle { enabled } => {
                self.queue
                    .update(cx, |queue, cx| queue.set_shuffle(*enabled, cx));
                QUEUE_PLAYBACK
            }
            Command::QueuePlayUpcoming {
                index,
                expected_revision,
            } => {
                self.guard(*index, *expected_revision, cx)?;
                self.playback
                    .update(cx, |playback, cx| playback.play_upcoming(*index, cx));
                QUEUE_PLAYBACK
            }
            Command::QueueRemoveUpcoming {
                index,
                expected_revision,
            } => {
                self.guard(*index, *expected_revision, cx)?;
                self.queue
                    .update(cx, |queue, cx| queue.remove_upcoming(*index, cx));
                QUEUE
            }
            Command::QueueMoveUpcoming {
                from,
                to,
                expected_revision,
            } => {
                self.guard(*from, *expected_revision, cx)?;
                let len = self.queue.read(cx).upcoming().len();
                if *to >= len {
                    return Err(ControlError::not_found(
                        "no queued track at that destination",
                    ));
                }
                if from == to {
                    return Err(ControlError::invalid_argument("the track is already there"));
                }
                self.queue
                    .update(cx, |queue, cx| queue.move_upcoming(*from, *to, cx));
                QUEUE
            }
            Command::QueueClearUpcoming { expected_revision } => {
                self.check_revision(*expected_revision, cx)?;
                self.queue.update(cx, |queue, cx| queue.clear_upcoming(cx));
                QUEUE
            }
        };
        let snapshot_revision = self.refresh(sections, cx);
        Ok(CommandReply {
            accepted: true,
            snapshot_revision,
        })
    }

    /// Rejects a queue command addressing a stale or missing entry, before it touches anything.
    fn guard(&self, index: usize, expected_revision: u64, cx: &App) -> Result<(), ControlError> {
        self.check_revision(expected_revision, cx)?;
        let len = self.queue.read(cx).upcoming().len();
        match index < len {
            true => Ok(()),
            false => Err(ControlError::not_found("no queued track at that index")),
        }
    }

    fn check_revision(&self, expected: u64, cx: &App) -> Result<(), ControlError> {
        match self.queue.read(cx).revision() == expected {
            true => Ok(()),
            false => Err(ControlError::conflict(
                "queue changed; request a new snapshot",
            )),
        }
    }

    /// Publishes a new revision when `sections` changed anything a snapshot shows, and reports
    /// the current revision either way. Deferred observers call this too; `same_state` drops
    /// their duplicate of what a command already published.
    fn refresh(&mut self, sections: &[Section], cx: &mut Context<Self>) -> u64 {
        let candidate = self.build(cx, self.snapshot_revision + 1);
        if same_state(&self.last, &candidate) {
            return self.snapshot_revision;
        }
        self.snapshot_revision += 1;
        self.publish.publish(candidate.clone());
        for section in sections {
            self.event_seq += 1;
            self.publish.emit(SequencedEvent {
                seq: self.event_seq,
                event: section_event(*section, &candidate),
            });
        }
        self.last = candidate;
        self.snapshot_revision
    }

    /// Emits the playing position for event subscribers. Positions are not state: the
    /// revision stands still, and nobody samples without subscribers.
    fn sample_position(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        if !should_sample(playback.state(), self.publish.has_receivers()) {
            return;
        }
        let event = ControlEvent::PositionChanged {
            snapshot_revision: self.snapshot_revision,
            position_ms: millis(playback.live_position()),
        };
        self.event_seq += 1;
        self.publish.emit(SequencedEvent {
            seq: self.event_seq,
            event,
        });
    }

    fn build(&self, cx: &App, revision: u64) -> AppSnapshot {
        snapshot(
            revision,
            self.playback.read(cx),
            self.queue.read(cx),
            self.session.read(cx),
        )
    }
}

/// The event one section of `snapshot` changed into.
fn section_event(section: Section, snapshot: &AppSnapshot) -> ControlEvent {
    let revision = snapshot.snapshot_revision;
    match section {
        Section::Playback => ControlEvent::PlaybackChanged {
            snapshot_revision: revision,
            status: snapshot.playback.status,
        },
        Section::Queue => ControlEvent::QueueChanged {
            snapshot_revision: revision,
            queue_revision: snapshot.queue.revision,
        },
        Section::Session => ControlEvent::SessionChanged {
            snapshot_revision: revision,
            status: snapshot.session.status,
        },
    }
}

/// The whole player state at `revision`, copied out of the live entities as plain data.
fn snapshot(revision: u64, playback: &Playback, queue: &Queue, session: &Session) -> AppSnapshot {
    let track = playback.track();
    let upcoming: Vec<TrackSummary> = queue
        .upcoming()
        .take(UPCOMING_LIMIT)
        .map(summarize)
        .collect();
    let manual_count = queue.manual().len().min(upcoming.len());
    let past = queue.past();
    let skip = past.len().saturating_sub(PAST_LIMIT);
    let past: Vec<TrackSummary> = past.skip(skip).map(summarize).collect();
    AppSnapshot {
        snapshot_revision: revision,
        playback: PlaybackSnapshot {
            status: playback_status(playback.state()),
            track: track.map(summarize),
            position_ms: millis(playback.live_position()),
            duration_ms: track.map(|track| millis(track.duration)),
            volume: volume(playback.volume()),
            repeat: repeat_mode(playback.repeat()),
            shuffle: queue.shuffle(),
        },
        queue: QueueSnapshot {
            revision: queue.revision(),
            past,
            current: queue.current().map(summarize),
            upcoming,
            suggested: queue
                .similar()
                .take(SUGGESTED_LIMIT)
                .map(summarize)
                .collect(),
            manual_count,
            shuffle: queue.shuffle(),
        },
        session: SessionSnapshot {
            status: session_status(session.state()),
            provider: session.provider_slug().map(str::to_owned),
        },
    }
}

/// Whether two snapshots describe the same state. The clock and the revision itself do not
/// count: the position moves while playing, and the revision only names the snapshot.
fn same_state(last: &AppSnapshot, next: &AppSnapshot) -> bool {
    last.playback.status == next.playback.status
        && last.playback.track == next.playback.track
        && last.playback.duration_ms == next.playback.duration_ms
        && last.playback.volume.to_bits() == next.playback.volume.to_bits()
        && last.playback.repeat == next.playback.repeat
        && last.playback.shuffle == next.playback.shuffle
        && last.queue == next.queue
        && last.session == next.session
}

/// Whether a position sample is worth emitting: the clock only moves while playing, and
/// only event subscribers hear it.
fn should_sample(state: &PlaybackState, receivers: bool) -> bool {
    receivers && *state == PlaybackState::Playing
}

/// Whole milliseconds in `duration`, saturating far past any real track length.
fn millis(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_mul(1000)
        .saturating_add(u64::from(duration.subsec_millis()))
}

/// The volume plugins see: the player's level, or silence when it is somehow not finite.
fn volume(level: f32) -> f32 {
    match level.is_finite() {
        true => level.clamp(0., 1.),
        false => 0.,
    }
}

/// The safe public summary of `track`. Local tracks keep no id: it is a filesystem path.
fn summarize(track: &Track) -> TrackSummary {
    let local = track.id.as_deref().is_some_and(music::is_local_id);
    TrackSummary {
        name: track.name.clone(),
        artists: track.artists.clone(),
        album: track.album.clone(),
        duration_ms: millis(track.duration),
        provider_track_id: match local {
            true => None,
            false => track.id.clone(),
        },
        is_local: local,
    }
}

/// The public player status. A failure keeps its reason inside.
fn playback_status(state: &PlaybackState) -> PlaybackStatus {
    match state {
        PlaybackState::Idle => PlaybackStatus::Idle,
        PlaybackState::Playing => PlaybackStatus::Playing,
        PlaybackState::Paused => PlaybackStatus::Paused,
        PlaybackState::Loading => PlaybackStatus::Loading,
        PlaybackState::Failed(_) => PlaybackStatus::Failed,
    }
}

fn repeat_mode(repeat: Repeat) -> RepeatMode {
    match repeat {
        Repeat::Off => RepeatMode::Off,
        Repeat::All => RepeatMode::All,
        Repeat::One => RepeatMode::One,
    }
}

fn repeat_requested(mode: RepeatMode) -> Repeat {
    match mode {
        RepeatMode::Off => Repeat::Off,
        RepeatMode::All => Repeat::All,
        RepeatMode::One => Repeat::One,
    }
}

/// The public session status. Profiles, prompts and failures stay inside.
fn session_status(state: &SessionState) -> SessionStatus {
    match state {
        SessionState::SignedOut => SessionStatus::SignedOut,
        SessionState::Restoring => SessionStatus::Restoring,
        SessionState::Authorizing(_) => SessionStatus::Authorizing,
        SessionState::SignedIn(_) => SessionStatus::SignedIn,
        SessionState::Offline(_) => SessionStatus::Offline,
        SessionState::Failed(_) => SessionStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use control::channel::RequestError;
    use control::{CommandReply, ErrorCode};
    use music::{
        InputSource, MusicProvider, PromptSink, ProviderSession, SignIn, SignInPrompt, UserProfile,
    };
    use storage::Database;

    use super::*;
    use crate::{AppSettings, Failure, Io};

    fn track(id: Option<&str>) -> Track {
        Track {
            id: id.map(str::to_owned),
            name: "Song".to_owned(),
            playable: true,
            artists: "Artist".to_owned(),
            artist_refs: Vec::new(),
            album: "Album".to_owned(),
            album_id: None,
            cover: None,
            duration: Duration::from_millis(180_000),
            added_at: None,
            added_by: None,
            playcount: None,
            popularity: 0,
            explicit: false,
            track_number: 0,
            disc_number: 0,
            tags: Vec::new(),
            languages: Vec::new(),
            credits: Vec::new(),
        }
    }

    #[test]
    fn local_track_ids_stay_inside() {
        let summary = summarize(&track(Some("local:/home/user/music/song.mp3")));

        assert!(summary.is_local);
        assert_eq!(summary.provider_track_id, None);
        assert_eq!(summary.name, "Song");
        assert_eq!(summary.duration_ms, 180_000);
    }

    #[test]
    fn redaction_fails_closed_on_any_local_prefix() {
        for id in [
            "local:/music/song.mp3",
            "local-album:/music",
            "local-artist:/music",
            "local-playlist:favorites",
        ] {
            let summary = summarize(&track(Some(id)));
            assert!(summary.is_local, "{id} reads as local");
            assert_eq!(summary.provider_track_id, None, "{id} keeps no id");
        }
    }

    #[test]
    fn remote_ids_pass_through() {
        let summary = summarize(&track(Some("spotify:track:abc")));

        assert!(!summary.is_local);
        assert_eq!(
            summary.provider_track_id.as_deref(),
            Some("spotify:track:abc")
        );

        let missing = summarize(&track(None));
        assert!(!missing.is_local);
        assert_eq!(missing.provider_track_id, None);
    }

    #[test]
    fn statuses_drop_reasons_prompts_and_profiles() {
        assert_eq!(
            playback_status(&PlaybackState::Failed("engine blew up".to_owned())),
            PlaybackStatus::Failed
        );
        assert_eq!(
            session_status(&SessionState::Authorizing(Some(SignInPrompt::Secret))),
            SessionStatus::Authorizing
        );
        assert_eq!(
            session_status(&SessionState::SignedIn(UserProfile {
                id: "user".to_owned(),
                display_name: "User".to_owned(),
                avatar: None,
            })),
            SessionStatus::SignedIn
        );
        let failure = Failure {
            problem: None,
            summary: "the account could not be reached".to_owned(),
            detail: None,
        };
        assert_eq!(
            session_status(&SessionState::Offline(failure.clone())),
            SessionStatus::Offline
        );
        assert_eq!(
            session_status(&SessionState::Failed(failure)),
            SessionStatus::Failed
        );
    }

    #[test]
    fn repeat_converts_both_ways() {
        for (repeat, mode) in [
            (Repeat::Off, RepeatMode::Off),
            (Repeat::All, RepeatMode::All),
            (Repeat::One, RepeatMode::One),
        ] {
            assert_eq!(repeat_mode(repeat), mode);
            assert_eq!(repeat_requested(mode), repeat);
        }
    }

    #[test]
    fn positions_sample_only_for_playing_subscribers() {
        assert!(should_sample(&PlaybackState::Playing, true));
        for state in [
            PlaybackState::Idle,
            PlaybackState::Paused,
            PlaybackState::Loading,
            PlaybackState::Failed("engine blew up".to_owned()),
        ] {
            assert!(!should_sample(&state, true));
        }
        assert!(!should_sample(&PlaybackState::Playing, false));
    }

    #[test]
    fn millis_counts_whole_milliseconds() {
        assert_eq!(millis(Duration::ZERO), 0);
        assert_eq!(millis(Duration::from_millis(90_500)), 90_500);
        assert_eq!(millis(Duration::from_secs(u64::MAX)), u64::MAX);
    }

    #[test]
    fn snapshot_volume_silences_garbage() {
        assert_eq!(volume(f32::NAN), 0.);
        assert_eq!(volume(f32::INFINITY), 0.);
        assert_eq!(volume(2.), 1.);
        assert_eq!(volume(-1.), 0.);
        assert_eq!(volume(0.6), 0.6);
    }

    fn pictured() -> AppSnapshot {
        AppSnapshot {
            snapshot_revision: 7,
            playback: PlaybackSnapshot {
                status: PlaybackStatus::Playing,
                track: None,
                position_ms: 1_000,
                duration_ms: None,
                volume: 0.6,
                repeat: RepeatMode::Off,
                shuffle: false,
            },
            queue: QueueSnapshot {
                revision: 3,
                past: Vec::new(),
                current: None,
                upcoming: Vec::new(),
                suggested: Vec::new(),
                manual_count: 0,
                shuffle: false,
            },
            session: SessionSnapshot {
                status: SessionStatus::SignedOut,
                provider: None,
            },
        }
    }

    #[test]
    fn same_state_ignores_only_the_clock_and_the_revision() {
        let mut moved = pictured();
        moved.snapshot_revision = 8;
        moved.playback.position_ms = 2_000;
        assert!(same_state(&pictured(), &moved));

        let mut stopped = pictured();
        stopped.playback.status = PlaybackStatus::Paused;
        assert!(!same_state(&pictured(), &stopped));

        let mut quieter = pictured();
        quieter.playback.volume = 0.5;
        assert!(!same_state(&pictured(), &quieter));

        let mut queued = pictured();
        queued.queue.revision = 4;
        assert!(!same_state(&pictured(), &queued));

        let mut signed_in = pictured();
        signed_in.session.status = SessionStatus::SignedIn;
        assert!(!same_state(&pictured(), &signed_in));
    }

    struct FakeProvider;

    #[async_trait::async_trait]
    impl MusicProvider for FakeProvider {
        fn name(&self) -> &'static str {
            "Test"
        }

        fn slug(&self) -> &'static str {
            "test"
        }

        fn sign_in_options(&self) -> Vec<SignIn> {
            Vec::new()
        }

        fn stored(&self) -> bool {
            false
        }

        async fn restore(&self) -> anyhow::Result<Option<ProviderSession>> {
            Ok(None)
        }

        async fn sign_in(
            &self,
            _method: SignIn,
            _prompt: PromptSink,
            _input: InputSource,
        ) -> anyhow::Result<ProviderSession> {
            anyhow::bail!("the test provider cannot sign in")
        }

        fn sign_out(&self) {}
    }

    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "sonora-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    struct Rig {
        io: Io,
        client: Client,
        queue: Entity<Queue>,
        root: PathBuf,
    }

    impl Rig {
        fn new(cx: &mut gpui::TestAppContext) -> Self {
            let root = scratch("control");
            let database = Database::at(root.join("state.sqlite"));
            let io = Io::new().expect("io runtime starts");
            let local = Arc::new(FakeProvider);
            let settings = cx.new(|_| AppSettings::load_at(root.join("settings.json"), database));
            let session =
                cx.new(|cx| Session::new(Vec::new(), local, settings.clone(), io.clone(), cx));
            let queue = cx.new(|cx| Queue::new(session.clone(), settings.clone(), cx));
            let playback =
                cx.new(|cx| Playback::new(session.clone(), queue.clone(), settings.clone(), cx));
            let client =
                cx.update(|cx| attach_for(playback.clone(), queue.clone(), session.clone(), cx));
            Self {
                io,
                client,
                queue,
                root,
            }
        }

        async fn run(&self, command: Command) -> Result<CommandReply, RequestError> {
            self.client.execute(command).await
        }

        async fn state(&self) -> AppSnapshot {
            self.client.snapshot().await.expect("snapshot arrives")
        }

        fn start(&self, ids: &[&str], cx: &mut gpui::TestAppContext) {
            let tracks: Vec<Track> = ids.iter().map(|id| track(Some(id))).collect();
            cx.update(|cx| {
                self.queue.update(cx, |queue, cx| {
                    queue.start(tracks, 0, cx);
                });
            });
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[gpui::test]
    async fn commands_drive_playback_and_report_back(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        // The io runtime, entered for the whole body: control timeouts need a tokio
        // context, and the test dispatcher forbids parking, so nothing here may wait on
        // another thread.
        let _handle = rig.io.handle();
        let _tokio = _handle.enter();
        let first = rig.state().await;
        assert_eq!(first.snapshot_revision, 0);
        assert_eq!(first.playback.status, PlaybackStatus::Idle);

        let reply = rig.run(Command::Toggle).await.expect("toggle runs");
        assert!(reply.accepted);
        assert_eq!(reply.snapshot_revision, 0);

        let reply = rig
            .run(Command::SetVolume { level: 0.6 })
            .await
            .expect("volume runs");
        assert!(reply.accepted);
        assert_eq!(reply.snapshot_revision, 1);
        assert_eq!(rig.state().await.playback.volume, 0.6);

        let reply = rig
            .run(Command::Seek { position_ms: 5_000 })
            .await
            .expect("seek runs without a track");
        assert!(reply.accepted);
    }

    #[gpui::test]
    async fn bad_volumes_stale_revisions_and_bad_indices_fail(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        // The io runtime, entered for the whole body: control timeouts need a tokio
        // context, and the test dispatcher forbids parking, so nothing here may wait on
        // another thread.
        let _handle = rig.io.handle();
        let _tokio = _handle.enter();

        let error = rig
            .run(Command::SetVolume { level: 2. })
            .await
            .expect_err("overfull volume fails");
        assert_eq!(
            error,
            RequestError::Rejected(ControlError::invalid_argument(
                "volume must be a finite level between 0.0 and 1.0"
            ))
        );

        let error = rig
            .run(Command::QueueRemoveUpcoming {
                index: 0,
                expected_revision: 999,
            })
            .await
            .expect_err("a stale revision fails");
        assert_eq!(
            error,
            RequestError::Rejected(ControlError::conflict(
                "queue changed; request a new snapshot"
            ))
        );

        let revision = rig.state().await.queue.revision;
        let error = rig
            .run(Command::QueueRemoveUpcoming {
                index: 9,
                expected_revision: revision,
            })
            .await
            .expect_err("a missing entry fails");
        assert!(matches!(
            error,
            RequestError::Rejected(ControlError {
                code: ErrorCode::NotFound,
                ..
            })
        ));

        rig.start(&["a", "b"], cx);
        let revision = rig.state().await.queue.revision;
        let error = rig
            .run(Command::QueueMoveUpcoming {
                from: 0,
                to: 0,
                expected_revision: revision,
            })
            .await
            .expect_err("a move to the same place fails");
        assert!(matches!(
            error,
            RequestError::Rejected(ControlError {
                code: ErrorCode::InvalidArgument,
                ..
            })
        ));
    }

    #[gpui::test]
    async fn queue_selection_reorder_removal_and_clear(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        // The io runtime, entered for the whole body: control timeouts need a tokio
        // context, and the test dispatcher forbids parking, so nothing here may wait on
        // another thread.
        let _handle = rig.io.handle();
        let _tokio = _handle.enter();
        rig.start(&["a", "b", "c", "d", "e"], cx);

        let ids = |snapshot: &AppSnapshot| {
            snapshot
                .queue
                .upcoming
                .iter()
                .map(|track| track.provider_track_id.clone())
                .collect::<Vec<_>>()
        };
        let first = rig.state().await;
        assert_eq!(
            first.queue.current.as_ref().unwrap().provider_track_id,
            Some("a".to_owned())
        );
        assert_eq!(first.queue.upcoming.len(), 4);

        let revision = first.queue.revision;
        let reply = rig
            .run(Command::QueuePlayUpcoming {
                index: 1,
                expected_revision: revision,
            })
            .await
            .expect("selection runs");
        assert!(reply.accepted);
        let played = rig.state().await;
        assert_eq!(
            played.queue.current.as_ref().unwrap().provider_track_id,
            Some("c".to_owned())
        );
        assert_eq!(played.queue.past.len(), 2);
        assert_eq!(ids(&played), [Some("d".to_owned()), Some("e".to_owned())]);

        let revision = played.queue.revision;
        rig.run(Command::QueueMoveUpcoming {
            from: 0,
            to: 1,
            expected_revision: revision,
        })
        .await
        .expect("reorder runs");
        let moved = rig.state().await;
        assert_eq!(ids(&moved), [Some("e".to_owned()), Some("d".to_owned())]);

        let revision = moved.queue.revision;
        rig.run(Command::QueueRemoveUpcoming {
            index: 0,
            expected_revision: revision,
        })
        .await
        .expect("removal runs");
        let removed = rig.state().await;
        assert_eq!(ids(&removed), [Some("d".to_owned())]);

        let revision = removed.queue.revision;
        rig.run(Command::QueueClearUpcoming {
            expected_revision: revision,
        })
        .await
        .expect("clear runs");
        let cleared = rig.state().await;
        assert!(cleared.queue.upcoming.is_empty());
        assert!(cleared.queue.current.is_some());
        assert!(!cleared.queue.past.is_empty());
    }

    #[gpui::test]
    async fn local_tracks_publish_without_ids(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        // The io runtime, entered for the whole body: control timeouts need a tokio
        // context, and the test dispatcher forbids parking, so nothing here may wait on
        // another thread.
        let _handle = rig.io.handle();
        let _tokio = _handle.enter();
        rig.start(
            &["local:/home/user/music/song.mp3", "spotify:track:abc"],
            cx,
        );

        let snapshot = rig.state().await;
        let current = snapshot.queue.current.as_ref().expect("a current track");
        assert!(current.is_local);
        assert_eq!(current.provider_track_id, None);

        let next = snapshot.queue.upcoming.first().expect("an upcoming track");
        assert!(!next.is_local);
        assert_eq!(next.provider_track_id.as_deref(), Some("spotify:track:abc"));
    }

    #[gpui::test]
    async fn ui_changes_reach_subscribers(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        // The io runtime, entered for the whole body: control timeouts need a tokio
        // context, and the test dispatcher forbids parking, so nothing here may wait on
        // another thread.
        let _handle = rig.io.handle();
        let _tokio = _handle.enter();
        rig.start(&["a", "b"], cx);
        cx.run_until_parked();

        let mut events = rig.client.subscribe();
        let revision = rig.state().await.queue.revision;
        cx.update(|cx| {
            rig.queue.update(cx, |queue, cx| {
                queue.prepend(track(Some("z")), cx);
            });
        });
        cx.run_until_parked();

        let event = events.try_recv().expect("an event arrives");
        assert!(event.seq > 0);
        match event.event {
            ControlEvent::QueueChanged {
                snapshot_revision,
                queue_revision,
            } => {
                assert!(queue_revision > revision);
                assert!(snapshot_revision > 0);
            }
            event => panic!("expected a queue change, saw {event:?}"),
        }
    }

    #[gpui::test]
    async fn lagging_subscribers_resync_from_a_snapshot(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        // The io runtime, entered for the whole body: control timeouts need a tokio
        // context, and the test dispatcher forbids parking, so nothing here may wait on
        // another thread.
        let _handle = rig.io.handle();
        let _tokio = _handle.enter();
        let mut events = rig.client.subscribe();

        for round in 0..40 {
            rig.run(Command::SetShuffle {
                enabled: round % 2 == 0,
            })
            .await
            .expect("shuffle toggles");
        }

        let error = events.try_recv().expect_err("the buffer overflowed");
        assert!(matches!(
            error,
            tokio::sync::broadcast::error::TryRecvError::Lagged(_)
        ));
        let fresh = rig.state().await;
        assert!(fresh.snapshot_revision > 0);
    }

    #[gpui::test]
    async fn idle_players_sample_nothing(cx: &mut gpui::TestAppContext) {
        let rig = Rig::new(cx);
        let _handle = rig.io.handle();
        let _tokio = _handle.enter();
        let mut events = rig.client.subscribe();

        cx.executor().advance_clock(Duration::from_secs(5));
        cx.run_until_parked();

        assert!(events.try_recv().is_err());
    }
}
