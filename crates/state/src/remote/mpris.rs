use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::c_void;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use gpui::{App, Task};
use music::{MediaKind, Track};
use tokio::sync::mpsc;
use zbus::fdo::{Properties, RequestNameFlags, RequestNameReply};
use zbus::interface;
use zbus::names::InterfaceName;
use zbus::zvariant::{Array, ObjectPath, OwnedValue, Str, Value};

use super::{BUS_NAME, Command, DISPLAY_NAME};
use crate::{PlaybackState, Repeat, Shelf, Sonora};

/// The desktop entry a native install ships, `sonora.desktop`.
const DESKTOP_ENTRY: &str = "sonora";
const NO_TRACK_PATH: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

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
    Seeked(i64),
}

/// Shared in-memory player state backing the MPRIS D-Bus interfaces.
struct MprisData {
    playback_status: &'static str,
    is_buffering: bool,
    position: Duration,
    position_updated_at: Instant,
    duration: Duration,
    metadata: HashMap<String, OwnedValue>,
    current_track_id: ObjectPath<'static>,
    volume: f64,
    shuffle: bool,
    loop_status: &'static str,
    can_play: bool,
    can_pause: bool,
    can_seek: bool,
    can_go_next: bool,
    can_go_previous: bool,
}

impl Default for MprisData {
    fn default() -> Self {
        let (metadata, current_track_id) = empty_metadata();
        Self {
            playback_status: "Stopped",
            is_buffering: false,
            position: Duration::ZERO,
            position_updated_at: Instant::now(),
            duration: Duration::ZERO,
            metadata,
            current_track_id,
            volume: 1.0,
            shuffle: false,
            loop_status: "None",
            can_play: false,
            can_pause: false,
            can_seek: false,
            can_go_next: false,
            can_go_previous: false,
        }
    }
}

/// The MPRIS `org.mpris.MediaPlayer2` root interface.
struct RootInterface {
    commands: mpsc::UnboundedSender<Command>,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    /// Brings Sonora's window to the foreground.
    async fn raise(&self) {
        self.commands.send(Command::Raise).ok();
    }

    /// Cleanly requests application termination.
    async fn quit(&self) {
        self.commands.send(Command::Quit).ok();
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn set_fullscreen(&self, _fullscreen: bool) {}

    #[zbus(property)]
    fn can_set_fullscreen(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> &str {
        DISPLAY_NAME
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        std::env::var("FLATPAK_ID").unwrap_or_else(|_| DESKTOP_ENTRY.to_owned())
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> &[&str] {
        &["file", "http", "https"]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> &[&str] {
        &[
            "audio/mpeg",
            "audio/flac",
            "audio/ogg",
            "audio/mp4",
            "audio/aac",
            "audio/x-wav",
            "audio/opus",
            "audio/x-flac",
            "audio/x-vorbis+ogg",
        ]
    }
}

/// The MPRIS `org.mpris.MediaPlayer2.Player` playback interface.
struct PlayerInterface {
    data: Arc<RwLock<MprisData>>,
    commands: mpsc::UnboundedSender<Command>,
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    async fn next(&self) {
        self.commands.send(Command::Next).ok();
    }

    async fn previous(&self) {
        self.commands.send(Command::Previous).ok();
    }

    async fn pause(&self) {
        self.commands.send(Command::Pause).ok();
    }

    async fn play_pause(&self) {
        self.commands.send(Command::Toggle).ok();
    }

    async fn stop(&self) {
        self.commands.send(Command::Pause).ok();
    }

    async fn play(&self) {
        self.commands.send(Command::Play).ok();
    }

    async fn seek(&self, offset: i64) {
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
        if track_id.as_str() == NO_TRACK_PATH || track_id != data.current_track_id {
            return;
        }
        if data.duration > Duration::ZERO && Duration::from_micros(position as u64) > data.duration
        {
            return;
        }
        self.commands
            .send(Command::Seek(Duration::from_micros(position as u64)))
            .ok();
    }

    async fn open_uri(&self, _uri: &str) {}

    #[zbus(signal)]
    async fn seeked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        position: i64,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.data
            .read()
            .map(|d| d.playback_status.to_string())
            .unwrap_or_else(|_| "Stopped".to_string())
    }

    #[zbus(property)]
    fn loop_status(&self) -> String {
        self.data
            .read()
            .map(|d| d.loop_status.to_string())
            .unwrap_or_else(|_| "None".to_string())
    }

    #[zbus(property)]
    async fn set_loop_status(&self, status: String) {
        let repeat = match status.as_str() {
            "Playlist" => Repeat::All,
            "Track" => Repeat::One,
            _ => Repeat::Off,
        };
        self.commands.send(Command::Repeat(repeat)).ok();
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn set_rate(&self, _rate: f64) {}

    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.data.read().map(|d| d.shuffle).unwrap_or(false)
    }

    #[zbus(property)]
    async fn set_shuffle(&self, shuffle: bool) {
        self.commands.send(Command::Shuffle(shuffle)).ok();
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
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
    async fn set_volume(&self, volume: f64) {
        self.commands
            .send(Command::Volume(volume.clamp(0.0, 1.0)))
            .ok();
    }

    /// Dynamic position queries: `emits_changed_signal = "false"` disables property caching
    /// in zbus, ensuring callers always get the live, high-precision interpolated position
    /// without flood of change signals.
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        let Ok(data) = self.data.read() else {
            return 0;
        };
        let pos = match data.playback_status {
            "Playing" if !data.is_buffering => {
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

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.data.read().map(|d| d.can_play).unwrap_or(false)
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.data.read().map(|d| d.can_pause).unwrap_or(false)
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.data.read().map(|d| d.can_seek).unwrap_or(false)
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.data.read().map(|d| d.can_go_next).unwrap_or(false)
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.data.read().map(|d| d.can_go_previous).unwrap_or(false)
    }
}

/// The MPRIS player controller published on the session D-Bus.
pub struct Controls {
    data: Arc<RwLock<MprisData>>,
    signals: mpsc::UnboundedSender<SignalEvent>,
    _server: Task<()>,
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

        let server_data = data.clone();
        let _server = cx.spawn(async move |_| {
            let root = RootInterface {
                commands: commands.clone(),
            };
            let player = PlayerInterface {
                data: server_data,
                commands,
            };

            let (conn, bus_name) = match register_mpris(root, player).await {
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
                        iface.playback_status_changed(emitter).await.ok();
                    }
                    SignalEvent::MetadataChanged => {
                        iface.metadata_changed(emitter).await.ok();
                    }
                    SignalEvent::VolumeChanged => {
                        iface.volume_changed(emitter).await.ok();
                    }
                    SignalEvent::ShuffleChanged => {
                        iface.shuffle_changed(emitter).await.ok();
                    }
                    SignalEvent::LoopStatusChanged => {
                        iface.loop_status_changed(emitter).await.ok();
                    }
                    SignalEvent::CapabilitiesChanged {
                        can_play_changed,
                        can_pause_changed,
                        can_seek_changed,
                        can_go_next_changed,
                        can_go_previous_changed,
                    } => {
                        let mut changed = HashMap::new();
                        if can_play_changed {
                            changed.insert("CanPlay", Value::from(iface.can_play()));
                        }
                        if can_pause_changed {
                            changed.insert("CanPause", Value::from(iface.can_pause()));
                        }
                        if can_seek_changed {
                            changed.insert("CanSeek", Value::from(iface.can_seek()));
                        }
                        if can_go_next_changed {
                            changed.insert("CanGoNext", Value::from(iface.can_go_next()));
                        }
                        if can_go_previous_changed {
                            changed.insert("CanGoPrevious", Value::from(iface.can_go_previous()));
                        }
                        if !changed.is_empty() {
                            Properties::properties_changed(
                                emitter,
                                InterfaceName::from_static_str_unchecked(PLAYER_INTERFACE),
                                changed,
                                Cow::Borrowed(&[]),
                            )
                            .await
                            .ok();
                        }
                    }
                    SignalEvent::Seeked(position) => {
                        PlayerInterface::seeked(emitter, position).await.ok();
                    }
                }
            }
        });

        Ok(Self {
            data,
            signals,
            _server,
        })
    }

    pub fn describe(&mut self, track: Option<&Track>, cover: Option<&str>, cx: &App) {
        let (metadata, current_track_id) = match track {
            Some(track) => metadata(track, cover, cx),
            None => empty_metadata(),
        };
        let duration = track.map(|t| t.duration).unwrap_or(Duration::ZERO);
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
            PlaybackState::Playing => "Playing",
            PlaybackState::Loading if wants_playing => "Playing",
            PlaybackState::Loading => "Paused",
            PlaybackState::Paused => "Paused",
            PlaybackState::Idle | PlaybackState::Failed(_) => "Stopped",
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
        if let Ok(mut data) = self.data.write() {
            data.position = at;
            data.position_updated_at = Instant::now();
        }
        let micros = at.as_micros().min(i64::MAX as u128) as i64;
        self.signals.send(SignalEvent::Seeked(micros)).ok();
    }

    pub fn set_volume(&mut self, level: f64) {
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
            Repeat::Off => "None",
            Repeat::All => "Playlist",
            Repeat::One => "Track",
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

    pub fn set_capabilities(
        &mut self,
        can_play: bool,
        can_pause: bool,
        can_seek: bool,
        can_go_next: bool,
        can_go_previous: bool,
    ) {
        let diff = if let Ok(mut data) = self.data.write() {
            let diff = (
                data.can_play != can_play,
                data.can_pause != can_pause,
                data.can_seek != can_seek,
                data.can_go_next != can_go_next,
                data.can_go_previous != can_go_previous,
            );
            data.can_play = can_play;
            data.can_pause = can_pause;
            data.can_seek = can_seek;
            data.can_go_next = can_go_next;
            data.can_go_previous = can_go_previous;
            diff
        } else {
            (false, false, false, false, false)
        };
        if diff.0 || diff.1 || diff.2 || diff.3 || diff.4 {
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
}

/// Registers the primary MPRIS name or falls back to an instance-specific name if already owned.
async fn register_mpris(
    root: RootInterface,
    player: PlayerInterface,
) -> zbus::Result<(zbus::Connection, String)> {
    let base_name = format!("org.mpris.MediaPlayer2.{BUS_NAME}");
    let conn = zbus::connection::Builder::session()?
        .serve_at(OBJECT_PATH, root)?
        .serve_at(OBJECT_PATH, player)?
        .build()
        .await?;

    let primary_acquired = match conn
        .request_name_with_flags(base_name.as_str(), RequestNameFlags::DoNotQueue.into())
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
    cx: &App,
) -> (HashMap<String, OwnedValue>, ObjectPath<'static>) {
    let mut map = HashMap::new();
    let track_id = track_id(track);

    if let Ok(val) = OwnedValue::try_from(Value::ObjectPath(track_id.clone())) {
        map.insert("mpris:trackid".to_string(), val);
    }
    if let Ok(val) = OwnedValue::try_from(Value::I64(
        track.duration.as_micros().min(i64::MAX as u128) as i64,
    )) {
        map.insert("mpris:length".to_string(), val);
    }
    if let Some(art_url) = cover
        && let Ok(val) = OwnedValue::try_from(Value::Str(Str::from(art_url)))
    {
        map.insert("mpris:artUrl".to_string(), val);
    }
    if let Ok(val) = OwnedValue::try_from(Value::Str(Str::from(track.name.as_str()))) {
        map.insert("xesam:title".to_string(), val);
    }
    if let Ok(val) = OwnedValue::try_from(Value::Str(Str::from(track.album.as_str()))) {
        map.insert("xesam:album".to_string(), val);
    }
    if let Some(id) = &track.id
        && let Some(url) = Sonora::global(cx)
            .session
            .read(cx)
            .client_of(Shelf::of(id))
            .and_then(|client| client.share_url(MediaKind::Track, id))
        && let Ok(val) = OwnedValue::try_from(Value::Str(Str::from(url)))
    {
        map.insert("xesam:url".to_string(), val);
    }

    let artists: Vec<String> = if !track.artist_refs.is_empty() {
        track.artist_refs.iter().map(|a| a.name.clone()).collect()
    } else if !track.artists.is_empty() {
        vec![track.artists.clone()]
    } else {
        Vec::new()
    };
    if !artists.is_empty() {
        let artist_strs: Vec<Str<'_>> = artists.iter().map(|s| Str::from(s.as_str())).collect();
        if let Ok(val) = OwnedValue::try_from(Value::Array(Array::from(artist_strs))) {
            map.insert("xesam:artist".to_string(), val);
        }
    }

    if track.track_number > 0
        && let Ok(val) = OwnedValue::try_from(Value::I32(track.track_number as i32))
    {
        map.insert("xesam:trackNumber".to_string(), val);
    }
    if track.disc_number > 0
        && let Ok(val) = OwnedValue::try_from(Value::I32(track.disc_number as i32))
    {
        map.insert("xesam:discNumber".to_string(), val);
    }
    if let Some(playcount) = track.playcount
        && let Ok(val) = OwnedValue::try_from(Value::I32(playcount.min(i32::MAX as u64) as i32))
    {
        map.insert("xesam:useCount".to_string(), val);
    }
    if !track.tags.is_empty() {
        let tag_strs: Vec<Str<'_>> = track.tags.iter().map(|s| Str::from(s.as_str())).collect();
        if let Ok(val) = OwnedValue::try_from(Value::Array(Array::from(tag_strs))) {
            map.insert("xesam:genre".to_string(), val);
        }
    }

    (map, track_id)
}

fn empty_metadata() -> (HashMap<String, OwnedValue>, ObjectPath<'static>) {
    let mut map = HashMap::new();
    let track_id = ObjectPath::from_static_str_unchecked(NO_TRACK_PATH);
    if let Ok(val) = OwnedValue::try_from(Value::ObjectPath(track_id.clone())) {
        map.insert("mpris:trackid".to_string(), val);
    }
    (map, track_id)
}

/// An object path identifying the track. Provider IDs may contain characters illegal in
/// D-Bus object paths, so the path is uniquely hashed into a hex identifier.
fn track_id(track: &Track) -> ObjectPath<'static> {
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
