//! Where the sound goes once it leaves Sonora. On macOS and Windows cpal's default device is
//! the real one. On Linux it is ALSA's route into PipeWire or PulseAudio, the same whichever
//! output plays, so `follow` asks the sound server which sink Sonora's stream is on instead.

use std::sync::LazyLock;

use tokio::sync::watch;

/// The stream Sonora has open, as last opened. `None` until the first output opens.
static OPENED: LazyLock<watch::Sender<Option<Stream>>> = LazyLock::new(|| watch::Sender::new(None));

/// An output device, as whatever knows it best names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// Stable across restarts and unique on this machine: the sink's name and its active port
    /// from a sound server, cpal's device id otherwise.
    pub id: String,
    /// What the user knows it as.
    pub name: String,
    /// The port of the device that plays, such as headphones or speakers on one card, when it
    /// has more than one.
    pub port: Option<String>,
    /// The format the device runs at, when a sound server says.
    pub format: Option<Format>,
}

/// The stream Sonora has open on cpal's default device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    pub device: Device,
    /// What Sonora sends it.
    pub format: Format,
}

/// A rate, a channel count and a sample format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u16,
    pub sample: Sample,
}

/// How each sample is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    Integer(u8),
    Float(u8),
    /// A format without a plain bit depth, such as a-law.
    Other,
}

/// The stream Sonora opened last, while it is open or until the next one opens.
pub fn stream() -> Option<Stream> {
    OPENED.borrow().clone()
}

/// Follows `stream` through every later open.
pub fn streams() -> watch::Receiver<Option<Stream>> {
    OPENED.subscribe()
}

/// Records the stream an output just opened.
pub(crate) fn opened(stream: Stream) {
    OPENED.send_replace(Some(stream));
}

impl Sample {
    pub(crate) fn of(format: cpal::SampleFormat) -> Self {
        use cpal::SampleFormat::*;
        match format {
            I8 | U8 => Self::Integer(8),
            I16 | U16 => Self::Integer(16),
            I24 | U24 => Self::Integer(24),
            I32 | U32 => Self::Integer(32),
            I64 | U64 => Self::Integer(64),
            F32 => Self::Float(32),
            F64 => Self::Float(64),
            _ => Self::Other,
        }
    }
}

#[cfg(target_os = "linux")]
pub use server::follow;

#[cfg(target_os = "linux")]
mod server {
    //! The sink Sonora plays on, read from PipeWire or PulseAudio over the PulseAudio native
    //! protocol, which `pipewire-pulse` speaks too. One socket carries the queries and the change
    //! events.

    use std::io::Cursor;
    use std::path::PathBuf;

    use anyhow::{Context as _, Result, bail};
    use pulseaudio::protocol::{
        self, AuthParams, AuthReply, ClientInfoList, Command, CommandReply, GetSinkInfo, Prop,
        Props, SampleFormat, ServerInfo, SetClientNameReply, SinkInfo, SinkInputInfoList,
        SubscriptionEvent, SubscriptionEventFacility, SubscriptionEventType, SubscriptionMask,
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::UnixStream;

    use super::{Device, Format, Sample};

    /// How long the events of one change are let pass before the server is asked again. A new
    /// stream shows on the default sink for a few milliseconds, until the server links it
    /// where it plays.
    const SETTLE: std::time::Duration = std::time::Duration::from_millis(250);

    /// How long the authentication cookie is. `pipewire-pulse` takes any cookie of this length
    /// and checks nothing else, so zeros stand in when there is no cookie file.
    const COOKIE: usize = 256;

    /// One open connection, past the handshake.
    struct Connection {
        socket: UnixStream,
        version: u16,
        seq: u32,
        /// Set when an event came in while waiting for a reply, so it is not lost.
        stirred: bool,
        /// The sink's index as last read, so its own changes can be told from others'.
        sink: Option<u32>,
        /// Sonora's own stream on the server as last read, `None` while it has none.
        input: Option<u32>,
        /// Whether Sonora has had a stream on this connection. Until it has, the default sink
        /// stands in; after, a stream gone for a moment, as on a reopen, leaves the output as is.
        played: bool,
    }

    /// Reports the sink Sonora's stream plays on, the server's default until it has one, and
    /// again whenever that sink, its port or its format changes. Returns once the server cannot
    /// be reached or hangs up; nothing is retried. Run it on tokio.
    pub async fn follow(mut report: impl FnMut(Device) + Send) -> Result<()> {
        let mut connection = Connection::open().await?;
        let mut last = None;
        loop {
            let device = connection.played_sink().await?;
            if last.as_ref() != Some(&device) {
                report(device.clone());
                last = Some(device);
            }
            connection.wait_for_change().await?;
            tokio::time::sleep(SETTLE).await;
        }
    }

    impl Connection {
        async fn open() -> Result<Self> {
            let socket = connect().await?;
            let mut connection = Self {
                socket,
                version: protocol::MAX_VERSION,
                seq: 0,
                stirred: false,
                sink: None,
                input: None,
                played: false,
            };
            let auth = AuthParams {
                version: protocol::MAX_VERSION,
                supports_shm: false,
                supports_memfd: false,
                cookie: cookie(),
            };
            let reply: AuthReply = connection.request(&Command::Auth(auth)).await?;
            connection.version = protocol::MAX_VERSION.min(reply.version);
            let mut props = Props::new();
            props.set(Prop::ApplicationName, c"Sonora");
            let _: SetClientNameReply = connection.request(&Command::SetClientName(props)).await?;
            connection
                .acknowledge(&Command::Subscribe(
                    SubscriptionMask::SINK
                        | SubscriptionMask::SINK_INPUT
                        | SubscriptionMask::SERVER,
                ))
                .await?;
            Ok(connection)
        }

        async fn played_sink(&mut self) -> Result<Device> {
            // The process id is on the client that owns a stream, not on the stream. A sandbox
            // can give another app the same id, so the client's name has to be ours too.
            let clients: ClientInfoList = self.request(&Command::GetClientInfoList).await?;
            let pid = format!("{}\0", std::process::id());
            let names = own_names();
            let own: Vec<u32> = clients
                .iter()
                .filter(|client| {
                    let named = |prop| {
                        let value = client.props.get(prop);
                        names.iter().any(|name| value == Some(name.as_bytes()))
                    };
                    client.props.get(Prop::ApplicationProcessId) == Some(pid.as_bytes())
                        && (named(Prop::ApplicationProcessBinary) || named(Prop::ApplicationName))
                })
                .map(|client| client.index)
                .collect();
            let inputs: SinkInputInfoList = self.request(&Command::GetSinkInputInfoList).await?;
            let mine = inputs.iter().find(|input| {
                input
                    .client_index
                    .is_some_and(|client| own.contains(&client))
            });
            self.input = mine.map(|input| input.index);
            self.played |= mine.is_some();
            let wanted = match (mine, self.sink) {
                (Some(input), _) => GetSinkInfo {
                    index: Some(input.sink_index),
                    name: None,
                },
                (None, Some(sink)) if self.played => GetSinkInfo {
                    index: Some(sink),
                    name: None,
                },
                (None, _) => {
                    let server: ServerInfo = self.request(&Command::GetServerInfo).await?;
                    let name = server
                        .default_sink_name
                        .context("the sound server has no default sink")?;
                    GetSinkInfo {
                        index: None,
                        name: Some(name),
                    }
                }
            };
            let sink: SinkInfo = self.request(&Command::GetSinkInfo(wanted)).await?;
            self.sink = Some(sink.index);
            Ok(device(&sink))
        }

        /// Waits for an event that can change what `played_sink` reads: the server's default
        /// moving, a stream coming or Sonora's moving or going, or the sink changing or going.
        async fn wait_for_change(&mut self) -> Result<()> {
            if std::mem::take(&mut self.stirred) {
                return Ok(());
            }
            loop {
                let frame = self.frame().await?;
                if let (_, Command::SubscribeEvent(event)) =
                    protocol::read_command_message(&mut Cursor::new(&frame), self.version)?
                    && self.concerns(&event)
                {
                    return Ok(());
                }
            }
        }

        fn concerns(&self, event: &SubscriptionEvent) -> bool {
            match event.event_facility {
                SubscriptionEventFacility::Server => true,
                SubscriptionEventFacility::Sink => {
                    event.event_type != SubscriptionEventType::New && event.index == self.sink
                }
                SubscriptionEventFacility::SinkInput => {
                    event.event_type == SubscriptionEventType::New || event.index == self.input
                }
                _ => false,
            }
        }

        /// Sends `command` and reads its reply, noting any event that arrives first.
        async fn request<T: CommandReply>(&mut self, command: &Command) -> Result<T> {
            let seq = self.send(command).await?;
            let frame = self.reply(seq).await?;
            let (_, reply) =
                protocol::read_reply_message::<T>(&mut Cursor::new(&frame), self.version)?;
            Ok(reply)
        }

        /// Sends a command whose reply carries nothing.
        async fn acknowledge(&mut self, command: &Command) -> Result<()> {
            let seq = self.send(command).await?;
            self.reply(seq).await.map(drop)
        }

        async fn send(&mut self, command: &Command) -> Result<u32> {
            let seq = self.seq;
            self.seq = self.seq.wrapping_add(1);
            let mut message = Vec::new();
            protocol::write_command_message(&mut message, seq, command, self.version)?;
            self.socket.write_all(&message).await?;
            Ok(seq)
        }

        /// The frame answering `seq`, an error reply turned into an error.
        async fn reply(&mut self, seq: u32) -> Result<Vec<u8>> {
            loop {
                let frame = self.frame().await?;
                match protocol::read_command_message(&mut Cursor::new(&frame), self.version)? {
                    (_, Command::SubscribeEvent(event)) => {
                        self.stirred |= self.concerns(&event);
                    }
                    (answered, Command::Error(error)) if answered == seq => {
                        bail!("the sound server refused: {error:?}")
                    }
                    (answered, Command::Reply) if answered == seq => return Ok(frame),
                    _ => {}
                }
            }
        }

        /// One whole message, its header included.
        async fn frame(&mut self) -> Result<Vec<u8>> {
            let mut frame = vec![0; protocol::DESCRIPTOR_SIZE];
            self.socket.read_exact(&mut frame).await?;
            let length = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
            if length > protocol::MAX_MEMBLOCKQ_LENGTH {
                bail!("the sound server sent a {length} byte message");
            }
            frame.resize(protocol::DESCRIPTOR_SIZE + length, 0);
            self.socket
                .read_exact(&mut frame[protocol::DESCRIPTOR_SIZE..])
                .await?;
            Ok(frame)
        }
    }

    /// The device a sink is, keyed by its name and the port that plays.
    fn device(sink: &SinkInfo) -> Device {
        let sink_name = sink.name.to_string_lossy();
        let port = sink.ports.get(sink.active_port);
        let id = match port {
            Some(port) => format!("{sink_name}#{}", port.name.to_string_lossy()),
            None => sink_name.to_string(),
        };
        let name = sink
            .description
            .as_ref()
            .map_or(sink_name.to_string(), |name| {
                name.to_string_lossy().into_owned()
            });
        let port = port
            .filter(|_| sink.ports.len() > 1)
            .map(|port| port.description.as_ref().unwrap_or(&port.name))
            .map(|port| port.to_string_lossy().into_owned());
        let spec = sink.sample_spec;
        let sample = match spec.format {
            SampleFormat::U8 => Sample::Integer(8),
            SampleFormat::S16Le | SampleFormat::S16Be => Sample::Integer(16),
            SampleFormat::S24Le
            | SampleFormat::S24Be
            | SampleFormat::S24In32Le
            | SampleFormat::S24In32Be => Sample::Integer(24),
            SampleFormat::S32Le | SampleFormat::S32Be => Sample::Integer(32),
            SampleFormat::Float32Le | SampleFormat::Float32Be => Sample::Float(32),
            _ => Sample::Other,
        };
        let format = (spec.sample_rate > 0 && spec.channels > 0).then_some(Format {
            rate: spec.sample_rate,
            channels: u16::from(spec.channels),
            sample,
        });
        Device {
            id,
            name,
            port,
            format,
        }
    }

    /// The names a client of this process goes by, as its binary or application name, nul
    /// ended as the server sends them: the executable's file name and the name it was run as.
    fn own_names() -> Vec<String> {
        let exe = std::env::current_exe().ok();
        let run = std::env::args_os().next().map(PathBuf::from);
        [exe, run]
            .into_iter()
            .flatten()
            .filter_map(|path| Some(format!("{}\0", path.file_name()?.to_str()?)))
            .collect()
    }

    /// The server's socket, found the way libpulse looks for it: `PULSE_SERVER` when it names
    /// a local socket, then the runtime directories, each tried in turn.
    async fn connect() -> Result<UnixStream> {
        let server = std::env::var("PULSE_SERVER").ok();
        let named = server
            .as_deref()
            .and_then(|server| server.strip_prefix("unix:"))
            .map(PathBuf::from);
        let runtime = [
            std::env::var_os("PULSE_RUNTIME_PATH").map(|path| PathBuf::from(path).join("native")),
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(|path| PathBuf::from(path).join("pulse/native")),
        ];
        for path in named.into_iter().chain(runtime.into_iter().flatten()) {
            if let Ok(socket) = UnixStream::connect(&path).await {
                return Ok(socket);
            }
        }
        bail!("no sound server socket answers")
    }

    /// The authentication cookie from where libpulse keeps it, or zeros.
    fn cookie() -> Vec<u8> {
        let home = dirs::home_dir();
        let candidates = [
            std::env::var_os("PULSE_COOKIE").map(PathBuf::from),
            std::env::var_os("XDG_CONFIG_HOME")
                .map(|config| PathBuf::from(config).join("pulse/cookie")),
            home.as_ref().map(|home| home.join(".config/pulse/cookie")),
            home.map(|home| home.join(".pulse-cookie")),
        ];
        candidates
            .into_iter()
            .flatten()
            .filter_map(|path| std::fs::read(path).ok())
            .find(|cookie| cookie.len() == COOKIE)
            .unwrap_or_else(|| vec![0; COOKIE])
    }
}
