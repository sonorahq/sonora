//! Google Cast devices on the local network: finding them over mDNS and talking to them over the
//! Cast v2 channel. Nothing here knows a provider. A provider launches its own receiver app on a
//! device and hands playback to it through whatever that app speaks, so the device plays on
//! without Sonora.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use music::connect::DeviceKind;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

/// The service every Cast device announces itself as.
const SERVICE: &str = "_googlecast._tcp.local.";
/// What a Cast group reports as its model. A group plays only what the Default Media Receiver
/// plays, never a provider's own receiver.
const GROUP_MODEL: &str = "Google Cast Group";
/// The `ca` capability bit of a device with a screen.
const VIDEO_OUT: u32 = 1;
const SENDER: &str = "sender-0";
const RECEIVER: &str = "receiver-0";
const CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
const HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const RECEIVER_NS: &str = "urn:x-cast:com.google.cast.receiver";
/// How long a device has to answer one request before it counts as gone.
const ANSWER: Duration = Duration::from_secs(10);
const CONNECT: Duration = Duration::from_secs(5);
/// A Cast message over this is refused, so a broken length cannot make the reader allocate
/// without bound.
const LARGEST: usize = 64 * 1024;

/// A Cast device as its mDNS announcement describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// The device's own id, which stays the same across addresses and restarts.
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub address: SocketAddr,
}

/// What the network said about a Cast device.
pub(crate) enum Seen {
    Found(Found),
    /// The device with this id left.
    Lost(String),
}

/// A provider's receiver app running on a device.
#[derive(Clone, Debug)]
pub(crate) struct Running {
    /// The address the app's own messages go to.
    pub transport: String,
}

/// Browses the network for Cast devices until the returned daemon shuts down, and reports each
/// one that comes or goes. Cast groups are left out.
pub(crate) fn browse(seen: UnboundedSender<Seen>) -> Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new().context("cannot start mdns discovery")?;
    let events = daemon
        .browse(SERVICE)
        .context("cannot browse for cast devices")?;
    tokio::spawn(async move {
        // the fullname a removal names, against the device id it was found under
        let mut ids: HashMap<String, String> = HashMap::new();
        while let Ok(event) = events.recv_async().await {
            let sent = match event {
                ServiceEvent::ServiceResolved(service) => {
                    let txt = &service.txt_properties;
                    let text = |key: &str| txt.get_property_val_str(key).unwrap_or("").to_owned();
                    if text("md") == GROUP_MODEL {
                        continue;
                    }
                    let address = service
                        .addresses
                        .iter()
                        .map(|ip| ip.to_ip_addr())
                        .min_by_key(|ip| !matches!(ip, IpAddr::V4(_)));
                    let Some(address) = address else {
                        continue;
                    };
                    let id = Some(text("id"))
                        .filter(|id| !id.is_empty())
                        .unwrap_or_else(|| service.fullname.clone());
                    let name = Some(text("fn"))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| text("md"));
                    let kind = match text("ca").parse::<u32>().unwrap_or(0) & VIDEO_OUT {
                        0 => DeviceKind::Speaker,
                        _ => DeviceKind::Tv,
                    };
                    ids.insert(service.fullname.clone(), id.clone());
                    seen.send(Seen::Found(Found {
                        id,
                        name,
                        kind,
                        address: SocketAddr::new(address, service.port),
                    }))
                }
                ServiceEvent::ServiceRemoved(_, fullname) => match ids.remove(&fullname) {
                    Some(id) => seen.send(Seen::Lost(id)),
                    None => continue,
                },
                _ => continue,
            };
            if sent.is_err() {
                break;
            }
        }
    });
    Ok(daemon)
}

/// One open Cast v2 channel to a device: TLS to its Cast port, carrying length-prefixed
/// `CastMessage` protobufs. It answers the device's heartbeat while it waits for an answer, and
/// is meant for a short exchange rather than to stay open.
pub(crate) struct Channel {
    stream: TlsStream<TcpStream>,
    request: u64,
}

impl Channel {
    /// Opens the channel and connects to the device's receiver.
    pub async fn open(address: SocketAddr) -> Result<Self> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .context("cannot pick the tls versions for cast")?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SelfSigned(provider)))
            .with_no_client_auth();
        let tcp = tokio::time::timeout(CONNECT, TcpStream::connect(address))
            .await
            .context("the cast device did not answer in time")?
            .context("cannot reach the cast device")?;
        // a device that takes the connection and never finishes the handshake would otherwise
        // hold up every request behind this one
        tokio::time::timeout(ANSWER, async {
            let stream = TlsConnector::from(Arc::new(config))
                .connect(ServerName::IpAddress(address.ip().into()), tcp)
                .await
                .context("cannot open tls to the cast device")?;
            let mut channel = Self { stream, request: 0 };
            channel
                .send(RECEIVER, CONNECTION, &json!({ "type": "CONNECT" }))
                .await?;
            Ok(channel)
        })
        .await
        .context("the cast device did not finish connecting in time")?
    }

    /// Whether the device can run the receiver app `app`.
    pub async fn available(&mut self, app: &str) -> Result<bool> {
        let answer = self
            .ask(
                RECEIVER,
                RECEIVER_NS,
                json!({ "type": "GET_APP_AVAILABILITY", "appId": [app] }),
            )
            .await?;
        Ok(answer["availability"][app] == "APP_AVAILABLE")
    }

    /// Starts the receiver app `app`, or finds it already running, and connects to it once it
    /// listens on `namespace`. A freshly launched app is listed before it listens, and a
    /// message sent to it before then goes unanswered.
    pub async fn launch(&mut self, app: &str, namespace: &str) -> Result<Running> {
        let id = self.next();
        self.send(
            RECEIVER,
            RECEIVER_NS,
            &json!({ "type": "LAUNCH", "appId": app, "requestId": id }),
        )
        .await?;
        let running = tokio::time::timeout(ANSWER, async {
            loop {
                let (_, space, message) = self.receive().await?;
                if space != RECEIVER_NS {
                    continue;
                }
                match message["type"].as_str() {
                    Some("LAUNCH_ERROR") => {
                        anyhow::bail!("the cast device would not launch the app: {message}")
                    }
                    Some("RECEIVER_STATUS") => {
                        if let Some(running) = running(&message, app, namespace) {
                            return Ok(running);
                        }
                    }
                    _ => {}
                }
            }
        })
        .await
        .context("the cast device did not launch the app in time")??;
        self.send(
            &running.transport,
            CONNECTION,
            &json!({ "type": "CONNECT" }),
        )
        .await?;
        Ok(running)
    }

    /// Sends `message` to a running app on its `namespace` and waits for the reply whose type is
    /// `reply`.
    pub async fn app_request(
        &mut self,
        app: &Running,
        namespace: &str,
        message: Value,
        reply: &str,
    ) -> Result<Value> {
        self.send(&app.transport, namespace, &message).await?;
        tokio::time::timeout(ANSWER, async {
            loop {
                let (source, space, message) = self.receive().await?;
                if source == app.transport && space == namespace && message["type"] == reply {
                    return Ok(message);
                }
            }
        })
        .await
        .context("the cast app did not answer in time")?
    }

    /// Sends a receiver request with the next request id and waits for its answer.
    async fn ask(&mut self, to: &str, namespace: &str, mut message: Value) -> Result<Value> {
        let id = self.next();
        message["requestId"] = id.into();
        self.send(to, namespace, &message).await?;
        tokio::time::timeout(ANSWER, async {
            loop {
                let (_, space, message) = self.receive().await?;
                if space == namespace && message["requestId"] == id {
                    return Ok(message);
                }
            }
        })
        .await
        .context("the cast device did not answer in time")?
    }

    fn next(&mut self) -> u64 {
        self.request += 1;
        self.request
    }

    async fn send(&mut self, to: &str, namespace: &str, message: &Value) -> Result<()> {
        let body = encode(to, namespace, &message.to_string());
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend(body);
        // tls may hold the frame back until flushed, and the answer waited for would never come
        tokio::time::timeout(ANSWER, async {
            self.stream.write_all(&frame).await?;
            self.stream.flush().await
        })
        .await
        .context("the cast device did not take a request in time")?
        .context("cannot write to the cast device")
    }

    /// The next message that is not a heartbeat, as its source, namespace and JSON payload.
    async fn receive(&mut self) -> Result<(String, String, Value)> {
        loop {
            let mut length = [0; 4];
            self.stream
                .read_exact(&mut length)
                .await
                .context("cannot read from the cast device")?;
            let length = u32::from_be_bytes(length) as usize;
            anyhow::ensure!(
                length <= LARGEST,
                "the cast device sent {length} bytes at once"
            );
            let mut body = vec![0; length];
            self.stream
                .read_exact(&mut body)
                .await
                .context("cannot read from the cast device")?;
            let Some((source, namespace, payload)) = decode(&body) else {
                continue;
            };
            let Ok(message) = serde_json::from_str::<Value>(&payload) else {
                continue;
            };
            if namespace == HEARTBEAT {
                if message["type"] == "PING" {
                    self.send(&source, HEARTBEAT, &json!({ "type": "PONG" }))
                        .await?;
                }
                continue;
            }
            return Ok((source, namespace, message));
        }
    }
}

/// Accepts the self-signed certificate every Cast device presents, while still checking that
/// the handshake was signed by its key. Nothing better is possible without Google's device
/// authentication, which only matters to a device that has to prove it is genuine.
#[derive(Debug)]
struct SelfSigned(Arc<CryptoProvider>);

impl ServerCertVerifier for SelfSigned {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// The app `app` in a receiver status, when it runs and listens on `namespace`.
fn running(status: &Value, app: &str, namespace: &str) -> Option<Running> {
    let found = status["status"]["applications"]
        .as_array()?
        .iter()
        .find(|running| running["appId"] == app)?;
    found["namespaces"]
        .as_array()?
        .iter()
        .find(|listened| listened["name"] == namespace)?;
    Some(Running {
        transport: found["transportId"].as_str()?.to_owned(),
    })
}

/// A `CastMessage` from this sender, with a string payload. Its fields in order: the protocol
/// version, source, destination, namespace, payload type and the payload.
fn encode(to: &str, namespace: &str, payload: &str) -> Vec<u8> {
    let mut out = vec![0x08, 0x00];
    for (tag, text) in [(0x12, SENDER), (0x1a, to), (0x22, namespace)] {
        out.push(tag);
        varint(&mut out, text.len() as u64);
        out.extend_from_slice(text.as_bytes());
    }
    out.extend([0x28, 0x00, 0x32]);
    varint(&mut out, payload.len() as u64);
    out.extend_from_slice(payload.as_bytes());
    out
}

/// The source, namespace and string payload of a `CastMessage`, or `None` for one that carries
/// a binary payload or does not parse.
fn decode(mut body: &[u8]) -> Option<(String, String, String)> {
    let (mut source, mut namespace, mut payload) = (None, None, None);
    while !body.is_empty() {
        let key = read_varint(&mut body)?;
        match key & 7 {
            0 => {
                read_varint(&mut body)?;
            }
            1 => body = body.get(8..)?,
            2 => {
                let length = read_varint(&mut body)? as usize;
                let value = body.get(..length)?;
                body = &body[length..];
                let text = || String::from_utf8(value.to_vec()).ok();
                match key >> 3 {
                    2 => source = text(),
                    4 => namespace = text(),
                    6 => payload = text(),
                    _ => {}
                }
            }
            5 => body = body.get(4..)?,
            _ => return None,
        }
    }
    Some((source?, namespace?, payload?))
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_varint(body: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = body.split_first()?;
        *body = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}
