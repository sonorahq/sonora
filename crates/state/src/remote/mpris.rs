use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::c_void;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use gpui::App;
use music::{MediaKind, Track};
use tokio::sync::mpsc;
use zbus::fdo::{Properties, RequestNameFlags, RequestNameReply};
use zbus::interface;
use zbus::names::InterfaceName;
use zbus::{Array, ObjectPath, OwnedVariant, Str, Variant};

use super::{BUS_NAME, Capabilities, Command, DISPLAY_NAME};
use crate::{Io, PlaybackState, Repeat, Shelf, Sonora};

/// The desktop entry a native install ships, `sonora.desktop`.
static DESKTOP_ENTRY: LazyLock<String> =
    LazyLock::new(|| std::env::var("FLATPAK_ID").unwrap_or_else(|_| "sonora".to_owned()));
const NO_TRACK_PATH: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum PlaybackStatus {
    #[default]
    Stopped,
    Playing,
    Paused,
}

impl PlaybackStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "Stopped",
            Self::Playing => "Playing",
            Self::Paused => "Paused",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum LoopStatus {
    #[default]
    None,
    Track,
    Playlist,
}

impl LoopStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Track => "Track",
            Self::Playlist => "Playlist",
        }
    }
}

/// Internal signal events dispatched to emit D-Bus `PropertiesChanged` and `Seeked` signals.
enum SignalEvent {
    PlaybackChanged,
    MetadataChanged,
    VolumeChanged,
    ShuffleChanged,
    LoopStatusChanged,
    CapabilitiesChanged {
        can_play_changed: bool,
        can_pause_changed: bool,
        can_seek_changed: bool,
        can_go_next_changed: bool,
        can_go_previous_changed: bool,
    },
    Seeked {
        track_id: ObjectPath<'static>,
        position: i64,
    },
}

/// Shared in-memory player state backing the MPRIS D-Bus interfaces.
struct MprisData {
    playback_status: PlaybackStatus,
    is_buffering: bool,
    position: Duration,
    position_updated_at: Instant,
    duration: Duration,
    metadata: HashMap<String, OwnedVariant>,
    current_track_id: ObjectPath<'static>,
    volume: f64,
    shuffle: bool,
    loop_status: LoopStatus,
    capabilities: Capabilities,
}

impl Default for MprisData {
    fn default() -> Self {
        let (metadata, current_track_id) = empty_metadata();
        Self {
            playback_status: PlaybackStatus::Stopped,
            is_buffering: false,
            position: Duration::ZERO,
            position_updated_at: Instant::now(),
            duration: Duration::ZERO,
            metadata,
            current_track_id,
            volume: 1.0,
            shuffle: false,
            loop_status: LoopStatus::None,
            capabilities: Capabilities::default(),
        }
    }
}

/// The MPRIS `org.mpris.MediaPlayer2` root interface.
struct RootInterface {
    commands: mpsc::UnboundedSender<Command>,
}

#[interface(name = "org.mpris.MediaPlayer2", spawn = false)]
impl RootInterface {
    /// Brings Sonora's window to the foreground.
    async fn raise(&self) {
        self.commands.send(Command::Raise).ok();
    }

    /// Cleanly requests application termination.
    async fn quit(&self) {
        self.commands.send(Command::Quit).ok();
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn set_fullscreen(&self, _fullscreen: bool) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "Fullscreen cannot be set via MPRIS".into(),
        ))
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_set_fullscreen(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn identity(&self) -> &str {
        DISPLAY_NAME
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn desktop_entry(&self) -> &str {
        DESKTOP_ENTRY.as_str()
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn supported_uri_schemes(&self) -> &[&str] {
        &[]
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn supported_mime_types(&self) -> &[&str] {
        &[]
    }
}

/// The MPRIS `org.mpris.MediaPlayer2.Player` playback interface.
struct PlayerInterface {
    data: Arc<RwLock<MprisData>>,
    commands: mpsc::UnboundedSender<Command>,
}

#[interface(name = "org.mpris.MediaPlayer2.Player", spawn = false)]
impl PlayerInterface {
    async fn next(&self) {
        if self.data.read().is_ok_and(|d| !d.capabilities.can_go_next) {
            return;
        }
        self.commands.send(Command::Next).ok();
    }

    async fn previous(&self) {
        if self
            .data
            .read()
            .is_ok_and(|d| !d.capabilities.can_go_previous)
        {
            return;
        }
        self.commands.send(Command::Previous).ok();
    }

    async fn pause(&self) {
        if self.data.read().is_ok_and(|d| !d.capabilities.can_pause) {
            return;
        }
        self.commands.send(Command::Pause).ok();
    }

    async fn play_pause(&self) {
        if self
            .data
            .read()
            .is_ok_and(|d| !d.capabilities.can_play && !d.capabilities.can_pause)
        {
            return;
        }
        self.commands.send(Command::Toggle).ok();
    }

    async fn stop(&self) {
        if self.data.read().is_ok_and(|d| {
            d.playback_status == PlaybackStatus::Stopped || !d.capabilities.can_pause
        }) {
            return;
        }
        self.commands.send(Command::Stop).ok();
    }

    async fn play(&self) {
        if self.data.read().is_ok_and(|d| !d.capabilities.can_play) {
            return;
        }
        self.commands.send(Command::Play).ok();
    }

    async fn seek(&self, offset: i64) {
        if offset == 0 {
            return;
        }
        if self.data.read().is_ok_and(|d| !d.capabilities.can_seek) {
            return;
        }
        let step = Duration::from_micros(offset.unsigned_abs());
        match offset < 0 {
            true => self.commands.send(Command::Back(step)).ok(),
            false => self.commands.send(Command::Forward(step)).ok(),
        };
    }

    /// Sets playback position. Per MPRIS spec, if position is negative, greater than the track
    /// length, or track_id does not match the currently playing track, the call MUST be silently ignored.
    async fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        if position < 0 {
            return;
        }
        let Ok(data) = self.data.read() else {
            return;
        };
        if !data.capabilities.can_seek {
            return;
        }
        if track_id.as_str() == NO_TRACK_PATH || track_id != data.current_track_id {
            return;
        }
        if data.duration > Duration::ZERO && Duration::from_micros(position as u64) > data.duration
        {
            return;
        }
        self.commands
            .send(Command::SetPosition {
                track_id: track_id.into_owned(),
                at: Duration::from_micros(position as u64),
            })
            .ok();
    }

    async fn open_uri(&self, _uri: &str) {}

    #[zbus(signal)]
    async fn seeked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        position: i64,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> &'static str {
        self.data
            .read()
            .map(|d| d.playback_status.as_str())
            .unwrap_or("Stopped")
    }

    #[zbus(property)]
    fn loop_status(&self) -> &'static str {
        self.data
            .read()
            .map(|d| d.loop_status.as_str())
            .unwrap_or("None")
    }

    #[zbus(property)]
    async fn set_loop_status(&self, status: &str) -> zbus::fdo::Result<()> {
        let (repeat, loop_status) = match status {
            "Playlist" => (Repeat::All, LoopStatus::Playlist),
            "Track" => (Repeat::One, LoopStatus::Track),
            "None" => (Repeat::Off, LoopStatus::None),
            _ => {
                return Err(zbus::fdo::Error::InvalidArgs(format!(
                    "Invalid LoopStatus: {status}"
                )));
            }
        };
        if let Ok(mut data) = self.data.write() {
            data.loop_status = loop_status;
        }
        self.commands.send(Command::Repeat(repeat)).ok();
        Ok(())
    }

    #[zbus(property(emits_changed_signal = "false"))]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn set_rate(&self, rate: f64) -> zbus::fdo::Result<()> {
        if !rate.is_finite() || (rate - 1.0).abs() > f64::EPSILON {
            return Err(zbus::fdo::Error::NotSupported(
                "Playback rate cannot be changed".into(),
            ));
        }
        Ok(())
    }

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.data.read().map(|d| d.shuffle).unwrap_or(false)
    }

    #[zbus(property)]
    async fn set_shuffle(&self, shuffle: bool) -> zbus::fdo::Result<()> {
        if let Ok(mut data) = self.data.write() {
            data.shuffle = shuffle;
        }
        self.commands.send(Command::Shuffle(shuffle)).ok();
        Ok(())
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedVariant> {
        self.data
            .read()
            .map(|d| d.metadata.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.data.read().map(|d| d.volume).unwrap_or(1.0)
    }

    #[zbus(property)]
    async fn set_volume(&self, volume: f64) -> zbus::fdo::Result<()> {
        if !volume.is_finite() {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Volume must be a finite number".into(),
            ));
        }
        let volume = f64::from(volume.clamp(0.0, 1.0) as f32);
        if let Ok(mut data) = self.data.write() {
            data.volume = volume;
        }
        self.commands.send(Command::Volume(volume)).ok();
        Ok(())
    }

    /// Live position query: `emits_changed_signal = "false"` indicates to D-Bus clients that
    /// Position is not announced via change signals and should be read directly.
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        let Ok(data) = self.data.read() else {
            return 0;
        };
        let pos = match data.playback_status {
            PlaybackStatus::Playing if !data.is_buffering => {
                let elapsed = data.position_updated_at.elapsed();
                let total = data.position.saturating_add(elapsed);
                if data.duration > Duration::ZERO {
                    total.min(data.duration)
                } else {
                    total
                }
            }
            _ => data.position,
        };
        pos.as_micros().min(i64::MAX as u128) as i64
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.data
            .read()
            .map(|d| d.capabilities.can_play)
            .unwrap_or(false)
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.data
            .read()
            .map(|d| d.capabilities.can_pause)
            .unwrap_or(false)
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.data
            .read()
            .map(|d| d.capabilities.can_seek)
            .unwrap_or(false)
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.data
            .read()
            .map(|d| d.capabilities.can_go_next)
            .unwrap_or(false)
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.data
            .read()
            .map(|d| d.capabilities.can_go_previous)
            .unwrap_or(false)
    }
}

/// The MPRIS player controller published on the session D-Bus.
pub struct Controls {
    data: Arc<RwLock<MprisData>>,
    signals: mpsc::UnboundedSender<SignalEvent>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for Controls {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl Controls {
    /// Claims `org.mpris.MediaPlayer2.sonora` on the session bus (with fallback to an instance
    /// name if claimed), registers the Root and Player interfaces, and services updates.
    pub fn new(
        _hwnd: Option<*mut c_void>,
        commands: mpsc::UnboundedSender<Command>,
        cx: &mut App,
    ) -> Result<Self> {
        let data = Arc::new(RwLock::new(MprisData::default()));
        let (signals, mut receiver) = mpsc::unbounded_channel();

        let io = Io::global(cx);
        let server_data = data.clone();
        let handle = io.spawn(async move {
            let root = RootInterface {
                commands: commands.clone(),
            };
            let player = PlayerInterface {
                data: server_data,
                commands,
            };

            let (conn, bus_name) = match register_mpris(root, player, &mut receiver).await {
                Ok(res) => res,
                Err(error) => {
                    return log::warn!("remote: cannot register mpris player: {error:#}");
                }
            };

            log::info!("remote: registered mpris player at {bus_name}");

            let Ok(player_iface) = conn
                .object_server()
                .interface::<_, PlayerInterface>(OBJECT_PATH)
                .await
            else {
                return log::warn!("remote: cannot access mpris player interface");
            };

            while let Some(event) = receiver.recv().await {
                let iface = player_iface.get().await;
                let emitter = player_iface.signal_emitter();
                match event {
                    SignalEvent::PlaybackChanged => {
                        if let Err(e) = iface.playback_status_changed(emitter).await {
                            log::debug!("remote: mpris playback_status_changed error: {e:#}");
                        }
                    }
                    SignalEvent::MetadataChanged => {
                        if let Err(e) = iface.metadata_changed(emitter).await {
                            log::debug!("remote: mpris metadata_changed error: {e:#}");
                        }
                    }
                    SignalEvent::VolumeChanged => {
                        if let Err(e) = iface.volume_changed(emitter).await {
                            log::debug!("remote: mpris volume_changed error: {e:#}");
                        }
                    }
                    SignalEvent::ShuffleChanged => {
                        if let Err(e) = iface.shuffle_changed(emitter).await {
                            log::debug!("remote: mpris shuffle_changed error: {e:#}");
                        }
                    }
                    SignalEvent::LoopStatusChanged => {
                        if let Err(e) = iface.loop_status_changed(emitter).await {
                            log::debug!("remote: mpris loop_status_changed error: {e:#}");
                        }
                    }
                    SignalEvent::CapabilitiesChanged {
                        can_play_changed,
                        can_pause_changed,
                        can_seek_changed,
                        can_go_next_changed,
                        can_go_previous_changed,
                    } => {
                        let changed = if let Ok(data) = iface.data.read() {
                            let mut changed = HashMap::new();
                            if can_play_changed {
                                changed
                                    .insert("CanPlay", Variant::from(data.capabilities.can_play));
                            }
                            if can_pause_changed {
                                changed
                                    .insert("CanPause", Variant::from(data.capabilities.can_pause));
                            }
                            if can_seek_changed {
                                changed
                                    .insert("CanSeek", Variant::from(data.capabilities.can_seek));
                            }
                            if can_go_next_changed {
                                changed.insert(
                                    "CanGoNext",
                                    Variant::from(data.capabilities.can_go_next),
                                );
                            }
                            if can_go_previous_changed {
                                changed.insert(
                                    "CanGoPrevious",
                                    Variant::from(data.capabilities.can_go_previous),
                                );
                            }
                            changed
                        } else {
                            continue;
                        };
                        if let Err(e) = Properties::properties_changed(
                            emitter,
                            InterfaceName::from_static_str_unchecked(PLAYER_INTERFACE),
                            changed,
                            Cow::Borrowed(&[]),
                        )
                        .await
                        {
                            log::debug!("remote: mpris properties_changed error: {e:#}");
                        }
                    }
                    SignalEvent::Seeked { track_id, position } => {
                        let matches = iface
                            .data
                            .read()
                            .is_ok_and(|d| d.current_track_id == track_id);
                        if matches && let Err(e) = PlayerInterface::seeked(emitter, position).await
                        {
                            log::debug!("remote: mpris seeked error: {e:#}");
                        }
                    }
                }
            }
        });

        Ok(Self {
            data,
            signals,
            handle,
        })
    }

    pub fn describe(
        &mut self,
        track: Option<&Track>,
        cover: Option<&str>,
        total: Duration,
        cx: &App,
    ) {
        let duration = match total.is_zero() {
            true => track.map(|t| t.duration).unwrap_or(Duration::ZERO),
            false => total,
        };
        let (metadata, current_track_id) = match track {
            Some(track) => metadata(track, cover, duration, cx),
            None => empty_metadata(),
        };
        let changed = if let Ok(mut data) = self.data.write() {
            let changed = data.metadata != metadata;
            data.metadata = metadata;
            data.current_track_id = current_track_id;
            data.duration = duration;
            changed
        } else {
            false
        };
        if changed {
            self.signals.send(SignalEvent::MetadataChanged).ok();
        }
    }

    pub fn set_playback(
        &mut self,
        state: &PlaybackState,
        at: Duration,
        wants_playing: bool,
        is_seeking: bool,
    ) {
        let status = match state {
            PlaybackState::Playing => PlaybackStatus::Playing,
            PlaybackState::Loading if wants_playing => PlaybackStatus::Playing,
            PlaybackState::Loading | PlaybackState::Paused => PlaybackStatus::Paused,
            PlaybackState::Idle | PlaybackState::Failed(_) => PlaybackStatus::Stopped,
        };
        let is_buffering = (matches!(state, PlaybackState::Loading) && wants_playing) || is_seeking;
        let status_changed = if let Ok(mut data) = self.data.write() {
            let changed = data.playback_status != status;
            data.playback_status = status;
            data.is_buffering = is_buffering;
            data.position = at;
            data.position_updated_at = Instant::now();
            changed
        } else {
            false
        };
        if status_changed {
            self.signals.send(SignalEvent::PlaybackChanged).ok();
        }
    }

    pub fn seeked(&mut self, at: Duration) {
        let Ok(mut data) = self.data.write() else {
            return;
        };
        data.position = at;
        data.position_updated_at = Instant::now();
        let track_id = data.current_track_id.clone();
        drop(data);

        let micros = at.as_micros().min(i64::MAX as u128) as i64;
        self.signals
            .send(SignalEvent::Seeked {
                track_id,
                position: micros,
            })
            .ok();
    }

    pub fn set_volume(&mut self, level: f64) {
        if level.is_nan() {
            return;
        }
        let changed = if let Ok(mut data) = self.data.write() {
            let clamped = level.clamp(0.0, 1.0);
            if (data.volume - clamped).abs() > f64::EPSILON {
                data.volume = clamped;
                true
            } else {
                false
            }
        } else {
            false
        };
        if changed {
            self.signals.send(SignalEvent::VolumeChanged).ok();
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        let changed = if let Ok(mut data) = self.data.write() {
            let changed = data.shuffle != on;
            data.shuffle = on;
            changed
        } else {
            false
        };
        if changed {
            self.signals.send(SignalEvent::ShuffleChanged).ok();
        }
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        let status = match repeat {
            Repeat::Off => LoopStatus::None,
            Repeat::All => LoopStatus::Playlist,
            Repeat::One => LoopStatus::Track,
        };
        let changed = if let Ok(mut data) = self.data.write() {
            let changed = data.loop_status != status;
            data.loop_status = status;
            changed
        } else {
            false
        };
        if changed {
            self.signals.send(SignalEvent::LoopStatusChanged).ok();
        }
    }

    pub fn set_capabilities(&mut self, caps: Capabilities) {
        let diff = if let Ok(mut data) = self.data.write() {
            if data.capabilities == caps {
                return;
            }
            let prev = data.capabilities;
            data.capabilities = caps;
            (
                prev.can_play != caps.can_play,
                prev.can_pause != caps.can_pause,
                prev.can_seek != caps.can_seek,
                prev.can_go_next != caps.can_go_next,
                prev.can_go_previous != caps.can_go_previous,
            )
        } else {
            return;
        };
        self.signals
            .send(SignalEvent::CapabilitiesChanged {
                can_play_changed: diff.0,
                can_pause_changed: diff.1,
                can_seek_changed: diff.2,
                can_go_next_changed: diff.3,
                can_go_previous_changed: diff.4,
            })
            .ok();
    }
}

/// Registers the primary MPRIS name or falls back to an instance-specific name if already owned.
async fn register_mpris(
    root: RootInterface,
    player: PlayerInterface,
    receiver: &mut mpsc::UnboundedReceiver<SignalEvent>,
) -> zbus::Result<(zbus::Connection, String)> {
    let base_name = format!("org.mpris.MediaPlayer2.{BUS_NAME}");
    let conn = zbus::connection::Builder::session()
        .serve_at(OBJECT_PATH, root)
        .serve_at(OBJECT_PATH, player)
        .build()
        .await?;

    // Drain any startup events that queued up during initial registration, since the newly
    // exported interfaces already expose the current MprisData.
    while receiver.try_recv().is_ok() {}

    let primary_acquired = match conn
        .request_name_with_flags(base_name.as_str(), RequestNameFlags::DO_NOT_QUEUE)
        .await
    {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => true,
        Ok(reply) => {
            log::info!(
                "remote: primary mpris name {base_name} unavailable ({reply:?}), trying instance name"
            );
            false
        }
        Err(err) => {
            log::info!(
                "remote: primary mpris name {base_name} request failed ({err:#}), trying instance name"
            );
            false
        }
    };

    let bus_name = if primary_acquired {
        base_name
    } else {
        let instance_name = format!("{base_name}.instance{}", std::process::id());
        conn.request_name(instance_name.as_str()).await?;
        instance_name
    };
    Ok((conn, bus_name))
}

/// Populates a comprehensive XESAM metadata map adhering to the MPRIS v2 specification.
fn metadata(
    track: &Track,
    cover: Option<&str>,
    duration: Duration,
    cx: &App,
) -> (HashMap<String, OwnedVariant>, ObjectPath<'static>) {
    let mut map = HashMap::new();
    let track_id = track_id(track);

    map.insert(
        "mpris:trackid".to_owned(),
        OwnedVariant::from(track_id.clone()),
    );
    if !duration.is_zero() {
        let length = duration.as_micros().min(i64::MAX as u128) as i64;
        map.insert("mpris:length".to_owned(), OwnedVariant::from(length));
    }
    if let Some(art_url) = cover {
        map.insert(
            "mpris:artUrl".to_owned(),
            OwnedVariant::from(Str::from(art_url)),
        );
    }
    map.insert(
        "xesam:title".to_owned(),
        OwnedVariant::from(Str::from(track.name.as_str())),
    );
    if !track.album.is_empty() {
        map.insert(
            "xesam:album".to_owned(),
            OwnedVariant::from(Str::from(track.album.as_str())),
        );
    }
    if let Some(id) = &track.id
        && let Some(url) = Sonora::global(cx)
            .session
            .read(cx)
            .client_of(Shelf::of(id))
            .and_then(|client| client.share_url(MediaKind::Track, id))
    {
        map.insert("xesam:url".to_owned(), OwnedVariant::from(Str::from(url)));
    }

    let artist_strs: Vec<Str<'_>> = if !track.artist_refs.is_empty() {
        track
            .artist_refs
            .iter()
            .map(|a| Str::from(a.name.as_str()))
            .collect()
    } else if !track.artists.is_empty() {
        vec![Str::from(track.artists.as_str())]
    } else {
        Vec::new()
    };
    if !artist_strs.is_empty()
        && let Ok(val) = OwnedVariant::try_from(Array::from(artist_strs))
    {
        map.insert("xesam:artist".to_owned(), val);
    }

    if track.track_number > 0 {
        map.insert(
            "xesam:trackNumber".to_owned(),
            OwnedVariant::from(track.track_number.min(i32::MAX as u32) as i32),
        );
    }
    if track.disc_number > 0 {
        map.insert(
            "xesam:discNumber".to_owned(),
            OwnedVariant::from(track.disc_number.min(i32::MAX as u32) as i32),
        );
    }
    if let Some(playcount) = track.playcount {
        map.insert(
            "xesam:useCount".to_owned(),
            OwnedVariant::from(playcount.min(i32::MAX as u64) as i32),
        );
    }
    if !track.tags.is_empty() {
        let tag_strs: Vec<Str<'_>> = track.tags.iter().map(|s| Str::from(s.as_str())).collect();
        if let Ok(val) = OwnedVariant::try_from(Array::from(tag_strs)) {
            map.insert("xesam:genre".to_owned(), val);
        }
    }

    (map, track_id)
}

fn empty_metadata() -> (HashMap<String, OwnedVariant>, ObjectPath<'static>) {
    let mut map = HashMap::new();
    let track_id = ObjectPath::from_static_str_unchecked(NO_TRACK_PATH);
    map.insert(
        "mpris:trackid".to_owned(),
        OwnedVariant::from(track_id.clone()),
    );
    (map, track_id)
}

/// An object path identifying the track. Provider IDs may contain characters illegal in
/// D-Bus object paths, so the path is uniquely hashed into a hex identifier.
pub(super) fn track_id(track: &Track) -> ObjectPath<'static> {
    let mut hasher = DefaultHasher::new();
    if let Some(id) = &track.id {
        id.hash(&mut hasher);
    } else {
        track.name.hash(&mut hasher);
        track.artists.hash(&mut hasher);
        track.album.hash(&mut hasher);
        track.track_number.hash(&mut hasher);
    }
    let path = format!("/app/sonora/track/t{:016x}", hasher.finish());
    ObjectPath::try_from(path)
        .unwrap_or_else(|_| ObjectPath::from_static_str_unchecked(NO_TRACK_PATH))
}
