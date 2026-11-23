use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::{fmt, io};

use anyhow::{Context as _, Result};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use futures::{SinkExt as _, StreamExt as _};
use num_bigint::BigUint;
use prost::Message as _;
use ring::{
    hmac,
    rand::{SecureRandom as _, SystemRandom},
    signature,
};
use shannon::Shannon;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tokio_util::codec::{Decoder, Encoder, Framed};

use crate::Session;
use crate::proto::access_point::{
    APLoginFailed, APResponseMessage, APWelcome, AuthenticationType, BuildInfo, ClientHello,
    ClientProof, ClientResponseEncrypted, CpuFamily, Cryptosuite, ErrorCode, LoginCredentials,
    LoginCryptoDiffieHellmanHello, LoginCryptoDiffieHellmanResponse, LoginCryptoHelloUnion,
    LoginCryptoResponseUnion, Os, Platform, Product, ProductFlags, SystemInfo,
};

const PING: u8 = 0x04;
const PONG: u8 = 0x49;
const LOGIN: u8 = 0xab;
const WELCOME: u8 = 0xac;
const LOGIN_FAILED: u8 = 0xad;
const REQUEST_KEY: u8 = 0x0c;
const KEY: u8 = 0x0d;
const KEY_FAILED: u8 = 0x0e;
const PRODUCT: u8 = 0x50;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const KEY_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const PRODUCT_WAIT: Duration = Duration::from_secs(5);
const KEY_CACHE: usize = 128;

const PROTOCOL_VERSION: u64 = 124200290;
const MAX_EXCHANGE: usize = 64 * 1024;
const HEADER_LEN: usize = 3;
const MAC_LEN: usize = 4;

// Spotify's public signing key and the fixed DH group used by the access-point protocol.
const SERVER_KEY: [u8; 256] = [
    0xac, 0xe0, 0x46, 0x0b, 0xff, 0xc2, 0x30, 0xaf, 0xf4, 0x6b, 0xfe, 0xc3, 0xbf, 0xbf, 0x86, 0x3d,
    0xa1, 0x91, 0xc6, 0xcc, 0x33, 0x6c, 0x93, 0xa1, 0x4f, 0xb3, 0xb0, 0x16, 0x12, 0xac, 0xac, 0x6a,
    0xf1, 0x80, 0xe7, 0xf6, 0x14, 0xd9, 0x42, 0x9d, 0xbe, 0x2e, 0x34, 0x66, 0x43, 0xe3, 0x62, 0xd2,
    0x32, 0x7a, 0x1a, 0x0d, 0x92, 0x3b, 0xae, 0xdd, 0x14, 0x02, 0xb1, 0x81, 0x55, 0x05, 0x61, 0x04,
    0xd5, 0x2c, 0x96, 0xa4, 0x4c, 0x1e, 0xcc, 0x02, 0x4a, 0xd4, 0xb2, 0x0c, 0x00, 0x1f, 0x17, 0xed,
    0xc2, 0x2f, 0xc4, 0x35, 0x21, 0xc8, 0xf0, 0xcb, 0xae, 0xd2, 0xad, 0xd7, 0x2b, 0x0f, 0x9d, 0xb3,
    0xc5, 0x32, 0x1a, 0x2a, 0xfe, 0x59, 0xf3, 0x5a, 0x0d, 0xac, 0x68, 0xf1, 0xfa, 0x62, 0x1e, 0xfb,
    0x2c, 0x8d, 0x0c, 0xb7, 0x39, 0x2d, 0x92, 0x47, 0xe3, 0xd7, 0x35, 0x1a, 0x6d, 0xbd, 0x24, 0xc2,
    0xae, 0x25, 0x5b, 0x88, 0xff, 0xab, 0x73, 0x29, 0x8a, 0x0b, 0xcc, 0xcd, 0x0c, 0x58, 0x67, 0x31,
    0x89, 0xe8, 0xbd, 0x34, 0x80, 0x78, 0x4a, 0x5f, 0xc9, 0x6b, 0x89, 0x9d, 0x95, 0x6b, 0xfc, 0x86,
    0xd7, 0x4f, 0x33, 0xa6, 0x78, 0x17, 0x96, 0xc9, 0xc3, 0x2d, 0x0d, 0x32, 0xa5, 0xab, 0xcd, 0x05,
    0x27, 0xe2, 0xf7, 0x10, 0xa3, 0x96, 0x13, 0xc4, 0x2f, 0x99, 0xc0, 0x27, 0xbf, 0xed, 0x04, 0x9c,
    0x3c, 0x27, 0x58, 0x04, 0xb6, 0xb2, 0x19, 0xf9, 0xc1, 0x2f, 0x02, 0xe9, 0x48, 0x63, 0xec, 0xa1,
    0xb6, 0x42, 0xa0, 0x9d, 0x48, 0x25, 0xf8, 0xb3, 0x9d, 0xd0, 0xe8, 0x6a, 0xf9, 0x48, 0x4d, 0xa1,
    0xc2, 0xba, 0x86, 0x30, 0x42, 0xea, 0x9d, 0xb3, 0x08, 0x6c, 0x19, 0x0e, 0x48, 0xb3, 0x9d, 0x66,
    0xeb, 0x00, 0x06, 0xa2, 0x5a, 0xee, 0xa1, 0x1b, 0x13, 0x87, 0x3c, 0xd7, 0x19, 0xe6, 0x55, 0xbd,
];
const PRIME: &[u8] = &[
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xc9, 0x0f, 0xda, 0xa2, 0x21, 0x68, 0xc2, 0x34,
    0xc4, 0xc6, 0x62, 0x8b, 0x80, 0xdc, 0x1c, 0xd1, 0x29, 0x02, 0x4e, 0x08, 0x8a, 0x67, 0xcc, 0x74,
    0x02, 0x0b, 0xbe, 0xa6, 0x3b, 0x13, 0x9b, 0x22, 0x51, 0x4a, 0x08, 0x79, 0x8e, 0x34, 0x04, 0xdd,
    0xef, 0x95, 0x19, 0xb3, 0xcd, 0x3a, 0x43, 0x1b, 0x30, 0x2b, 0x0a, 0x6d, 0xf2, 0x5f, 0x14, 0x37,
    0x4f, 0xe1, 0x35, 0x6d, 0x6d, 0x51, 0xc2, 0x45, 0xe4, 0x85, 0xb5, 0x76, 0x62, 0x5e, 0x7e, 0xc6,
    0xf4, 0x4c, 0x42, 0xe9, 0xa6, 0x3a, 0x36, 0x20, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
];

type MediaKey = ([u8; 16], [u8; 20]);

/// Lazily opens the account's audio-key connection and records key throttling.
pub struct AudioKeys {
    session: Arc<Session>,
    device_id: String,
    connection: Mutex<Option<AccessPoint>>,
    throttled: AtomicBool,
}

impl AudioKeys {
    /// Binds a lazy audio-key connection to one account and device.
    pub fn new(session: Arc<Session>, device_id: String) -> Self {
        Self {
            session,
            device_id,
            connection: Mutex::default(),
            throttled: AtomicBool::default(),
        }
    }

    /// Opens the access point on the first native key request and reconnects after closure.
    pub async fn get(&self, track: [u8; 16], file: [u8; 20]) -> Result<[u8; 16]> {
        let point = {
            let mut held = self.connection.lock().await;
            if let Some(point) = held.as_ref().filter(|point| !point.is_closed()) {
                point.clone()
            } else {
                let session = &self.session;
                let (addresses, token) =
                    tokio::try_join!(session.access_points(), session.access_token())?;
                let point =
                    AccessPoint::connect(addresses, &token, session.username(), &self.device_id)
                        .await?;
                held.insert(point).clone()
            }
        };
        let key = point.audio_key(track, file).await;
        if key.is_ok() {
            self.throttled.store(false, Ordering::Relaxed);
        } else if key.as_ref().is_err_and(|error| {
            error.downcast_ref::<AudioKeyError>() == Some(&AudioKeyError::Throttled)
        }) {
            self.throttled.store(true, Ordering::Relaxed);
        }
        key
    }

    /// Whether Spotify throttled a key request since the last successful one.
    pub fn throttled(&self) -> bool {
        self.throttled.load(Ordering::Relaxed)
    }
}

/// A Spotify-authorized connection for audio keys and account attributes.
#[derive(Clone)]
struct AccessPoint(Arc<Inner>);

struct Inner {
    requests: mpsc::Sender<Request>,
    account: watch::Receiver<Option<bool>>,
    task: JoinHandle<()>,
}

struct Request {
    media: MediaKey,
    reply: oneshot::Sender<Result<[u8; 16]>>,
}

/// Spotify's rejection of an access-point login or audio-key request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioKeyError {
    Denied,
    Throttled,
    Premium,
    Region,
    Credentials,
    Refused,
    Network,
}

impl fmt::Display for AudioKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Denied => "Spotify denied the audio key",
            Self::Throttled => "Spotify throttled the audio-key request",
            Self::Premium => "Spotify requires a premium account",
            Self::Region => "Spotify restricted access in this region",
            Self::Credentials => "Spotify rejected the access-point credentials",
            Self::Refused => "Spotify refused the access-point login",
            Self::Network => "cannot connect to a Spotify access point",
        })
    }
}

impl std::error::Error for AudioKeyError {}

impl AccessPoint {
    async fn connect(
        addresses: &[String],
        token: &str,
        username: &str,
        device: &str,
    ) -> Result<Self> {
        let mut last = None;
        for address in addresses {
            let Some((host, port)) = address.rsplit_once(':') else {
                continue;
            };
            let Ok(port) = port.parse::<u16>() else {
                continue;
            };
            if !host.ends_with(".spotify.com")
                || !host
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
            {
                continue;
            }
            match timeout(CONNECT_TIMEOUT, login(host, port, token, username, device)).await {
                Ok(Ok(socket)) => {
                    let (write, read) = mpsc::channel(32);
                    let (account, mut latest) = watch::channel(None);
                    let task = tokio::spawn(async move {
                        if let Err(error) = serve(socket, read, account).await {
                            log::debug!("spotify: access point closed: {error:#}");
                        }
                    });
                    let point = Self(Arc::new(Inner {
                        requests: write,
                        account: latest.clone(),
                        task,
                    }));
                    match timeout(PRODUCT_WAIT, latest.wait_for(|state| state.is_some())).await {
                        Ok(Ok(account)) if *account == Some(false) => {
                            return Err(AudioKeyError::Premium.into());
                        }
                        Ok(Err(_)) => return Err(AudioKeyError::Network.into()),
                        _ => return Ok(point),
                    }
                }
                Ok(Err(error)) if error.is::<AudioKeyError>() => return Err(error),
                Ok(Err(error)) => last = Some(error),
                Err(error) => last = Some(error.into()),
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("Spotify returned no access point")))
            .context(AudioKeyError::Network)
    }

    fn is_closed(&self) -> bool {
        self.0.requests.is_closed()
    }

    /// Requests the key from the signed-in access point without sending it to the CDN.
    async fn audio_key(&self, track: [u8; 16], file: [u8; 20]) -> Result<[u8; 16]> {
        if *self.0.account.borrow() == Some(false) {
            return Err(AudioKeyError::Denied.into());
        }
        let (send, reply) = oneshot::channel();
        timeout(KEY_TIMEOUT, async {
            self.0
                .requests
                .send(Request {
                    media: (track, file),
                    reply: send,
                })
                .await
                .map_err(|_| anyhow::anyhow!("Spotify access point closed"))?;
            reply.await.context("Spotify access point closed")?
        })
        .await
        .context("Spotify did not return an audio key in time")?
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn login(host: &str, port: u16, token: &str, username: &str, device: &str) -> Result<Socket> {
    let tcp = TcpStream::connect((host, port)).await?;
    tcp.set_nodelay(true)?;
    let mut socket = handshake(tcp).await?;
    let cpu = match std::env::consts::ARCH {
        "x86" => CpuFamily::X86,
        "x86_64" => CpuFamily::X86_64,
        "aarch64" | "arm" => CpuFamily::Arm,
        _ => CpuFamily::Unknown,
    };
    let os = match std::env::consts::OS {
        "linux" => Os::Linux,
        "macos" => Os::Macos,
        "windows" => Os::Windows,
        _ => Os::Unknown,
    };
    let packet = ClientResponseEncrypted {
        login_credentials: LoginCredentials {
            typ: AuthenticationType::SpotifyToken as i32,
            access_token: Some(token.to_owned()),
        },
        system_info: SystemInfo {
            cpu_family: cpu as i32,
            os: os as i32,
            device_id: Some(device.to_owned()),
            system_information_string: Some("Sonora".to_owned()),
        },
        version_string: Some(concat!("Sonora ", env!("CARGO_PKG_VERSION")).to_owned()),
    };
    socket.send((LOGIN, packet.encode_to_vec())).await?;
    let (command, body) = socket
        .next()
        .await
        .context("Spotify closed the login connection")??;
    match command {
        WELCOME => {
            let welcome = APWelcome::decode(body)?;
            if !welcome.canonical_username.eq_ignore_ascii_case(username) {
                return Err(AudioKeyError::Credentials.into());
            }
            Ok(socket)
        }
        LOGIN_FAILED => Err(failure(APLoginFailed::decode(body)?.error_code())),
        _ => anyhow::bail!("unexpected Spotify login response"),
    }
}

fn failure(code: ErrorCode) -> anyhow::Error {
    let problem = match code {
        ErrorCode::TryAnotherAP => {
            return anyhow::anyhow!("Spotify requested another access point");
        }
        ErrorCode::TravelRestriction => AudioKeyError::Region,
        ErrorCode::PremiumAccountRequired => AudioKeyError::Premium,
        ErrorCode::BadCredentials | ErrorCode::CouldNotValidateCredentials => {
            AudioKeyError::Credentials
        }
        _ => AudioKeyError::Refused,
    };
    problem.into()
}

async fn serve(
    mut socket: Socket,
    mut requests: mpsc::Receiver<Request>,
    account: watch::Sender<Option<bool>>,
) -> Result<()> {
    let mut cache = HashMap::new();
    let mut next = 0_u32;
    let mut pending: Option<(u32, Request, Instant)> = None;
    let idle = tokio::time::sleep(IDLE_TIMEOUT);
    tokio::pin!(idle);

    loop {
        let expiry = pending.as_ref().map(|(_, _, expires)| *expires);
        tokio::select! {
            frame = socket.next() => {
                let (command, body) = frame.context("Spotify closed the access point")??;
                idle.as_mut().reset(Instant::now() + IDLE_TIMEOUT);
                match command {
                    PING => socket.send((PONG, vec![0; 4])).await?,
                    PRODUCT => {
                        if let Some(premium) = product(&body) {
                            account.send_replace(Some(premium));
                            if !premium {
                                cache.clear();
                            }
                        }
                    }
                    KEY | KEY_FAILED => {
                        let Some(sequence) = body.get(..4).and_then(|bytes| bytes.try_into().ok()).map(u32::from_be_bytes) else { continue };
                        if pending.as_ref().is_none_or(|(expected, _, _)| *expected != sequence) {
                            continue;
                        }
                        let (_, request, _) = pending.take().expect("matching pending audio key");
                        let result = match command {
                            KEY => body.get(4..20).and_then(|key| <[u8; 16]>::try_from(key).ok())
                                .context("Spotify returned an invalid audio key"),
                            _ if body.get(4..6) == Some([0, 1].as_slice()) => Err(AudioKeyError::Denied.into()),
                            _ => Err(AudioKeyError::Throttled.into()),
                        };
                        if let Ok(key) = &result {
                            if cache.len() >= KEY_CACHE {
                                cache.clear();
                            }
                            cache.insert(request.media, *key);
                        }
                        let _ = request.reply.send(result);
                    }
                    _ => {}
                }
            }
            request = requests.recv(), if pending.is_none() => {
                let Some(request) = request else { return Ok(()) };
                if request.reply.is_closed() {
                    continue;
                }
                if *account.borrow() == Some(false) {
                    let _ = request.reply.send(Err(AudioKeyError::Denied.into()));
                    continue;
                }
                if let Some(key) = cache.get(&request.media) {
                    let _ = request.reply.send(Ok(*key));
                    continue;
                }
                let (track, file) = request.media;
                let mut body = Vec::with_capacity(42);
                body.extend_from_slice(&file);
                body.extend_from_slice(&track);
                body.extend_from_slice(&next.to_be_bytes());
                body.extend_from_slice(&[0; 2]);
                socket.send((REQUEST_KEY, body)).await?;
                pending = Some((next, request, Instant::now() + KEY_TIMEOUT));
                next = next.wrapping_add(1);
            }
            () = async {
                if let Some(expiry) = expiry {
                    tokio::time::sleep_until(expiry).await;
                }
            }, if expiry.is_some() => {
                if let Some((_, request, _)) = pending.take() {
                    let _ = request.reply.send(Err(anyhow::anyhow!("Spotify audio-key request timed out")));
                }
            }
            () = &mut idle => anyhow::bail!("Spotify access point stopped responding"),
        }
    }
}

fn product(bytes: &[u8]) -> Option<bool> {
    let xml = std::str::from_utf8(bytes)
        .ok()?
        .trim_matches(|c: char| c.is_whitespace() || c == '\0');
    let document = roxmltree::Document::parse(xml).ok()?;
    let product = document
        .descendants()
        .find(|node| node.has_tag_name("type"))?
        .text()?
        .trim();
    (!product.is_empty()).then_some(product == "premium")
}

struct Codec {
    tx: Shannon,
    rx: Shannon,
    sent: u32,
    received: u32,
    body: Option<(u8, usize)>,
}

type Socket = Framed<TcpStream, Codec>;

/// Authenticates the access point before creating the encrypted packet transport.
async fn handshake(mut tcp: TcpStream) -> Result<Socket> {
    let mut entropy = [0; 95 + 16];
    SystemRandom::new()
        .fill(&mut entropy)
        .map_err(|_| anyhow::anyhow!("cannot generate Spotify handshake entropy"))?;
    let (private, nonce) = entropy.split_at(95);
    let modulus = BigUint::from_bytes_be(PRIME);
    let private = BigUint::from_bytes_be(private);
    let public = BigUint::from(2_u8).modpow(&private, &modulus);
    let hello = ClientHello {
        build_info: BuildInfo {
            product: Product::Client as i32,
            product_flags: vec![ProductFlags::None as i32],
            platform: platform() as i32,
            version: PROTOCOL_VERSION,
        },
        cryptosuites_supported: vec![Cryptosuite::Shannon as i32],
        login_crypto_hello: LoginCryptoHelloUnion {
            diffie_hellman: Some(LoginCryptoDiffieHellmanHello {
                gc: public.to_bytes_be(),
                server_keys_known: 1,
            }),
        },
        client_nonce: nonce.to_vec(),
        padding: Some(vec![0x1e]),
    };
    let mut transcript = framed(&[0, 4], &hello.encode_to_vec())?;
    tcp.write_all(&transcript).await?;
    let length = tcp.read_u32().await?;
    let size = usize::try_from(length)?
        .checked_sub(4)
        .filter(|size| *size <= MAX_EXCHANGE)
        .context("invalid Spotify handshake length")?;
    transcript.extend_from_slice(&length.to_be_bytes());
    let start = transcript.len();
    transcript.resize(start + size, 0);
    tcp.read_exact(&mut transcript[start..]).await?;
    let response = APResponseMessage::decode(&transcript[start..])?;
    if let Some(rejection) = response.login_failed.as_ref() {
        return Err(failure(rejection.error_code()));
    }
    let remote = response
        .challenge
        .and_then(|challenge| challenge.login_crypto_challenge.diffie_hellman)
        .context("Spotify omitted its handshake challenge")?;
    signature::RsaPublicKeyComponents {
        n: SERVER_KEY.as_slice(),
        e: &[1, 0, 1],
    }
    .verify(
        &signature::RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY,
        &remote.gs,
        &remote.gs_signature,
    )
    .map_err(|_| anyhow::anyhow!("Spotify access-point signature did not verify"))?;
    let public = BigUint::from_bytes_be(&remote.gs);
    let one = BigUint::from(1_u8);
    anyhow::ensure!(
        public > one && public < &modulus - &one,
        "invalid Spotify handshake public key"
    );
    let shared = public.modpow(&private, &modulus).to_bytes_be();
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &shared);
    let mut material = Vec::with_capacity(100);
    for index in 1..=5 {
        let mut mac = hmac::Context::with_key(&key);
        mac.update(&transcript);
        mac.update(&[index]);
        material.extend_from_slice(mac.sign().as_ref());
    }
    let proof = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &material[..20]),
        &transcript,
    );
    let answer = ClientProof {
        login_crypto_response: LoginCryptoResponseUnion {
            diffie_hellman: Some(LoginCryptoDiffieHellmanResponse {
                hmac: proof.as_ref().to_vec(),
            }),
        },
        pow_response: (),
        crypto_response: (),
    };
    tcp.write_all(&framed(&[], &answer.encode_to_vec())?)
        .await?;
    Ok(Framed::new(
        tcp,
        Codec {
            tx: Shannon::new(&material[20..52]),
            rx: Shannon::new(&material[52..84]),
            sent: 0,
            received: 0,
            body: None,
        },
    ))
}

impl Encoder<(u8, Vec<u8>)> for Codec {
    type Error = io::Error;

    fn encode(&mut self, (command, payload): (u8, Vec<u8>), out: &mut BytesMut) -> io::Result<()> {
        let length = u16::try_from(payload.len()).map_err(io::Error::other)?;
        let start = out.len();
        out.put_u8(command);
        out.put_u16(length);
        out.extend_from_slice(&payload);
        self.tx.nonce_u32(self.sent);
        self.sent = self
            .sent
            .checked_add(1)
            .ok_or_else(|| io::Error::other("Spotify send nonce exhausted"))?;
        self.tx.encrypt(&mut out[start..]);
        let mut mac = [0; MAC_LEN];
        self.tx.finish(&mut mac);
        out.extend_from_slice(&mac);
        Ok(())
    }
}

impl Decoder for Codec {
    type Item = (u8, Bytes);
    type Error = io::Error;

    fn decode(&mut self, input: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        if self.body.is_none() {
            if input.len() < HEADER_LEN {
                return Ok(None);
            }
            let mut header = input.split_to(HEADER_LEN);
            self.rx.nonce_u32(self.received);
            self.received = self
                .received
                .checked_add(1)
                .ok_or_else(|| io::Error::other("Spotify receive nonce exhausted"))?;
            self.rx.decrypt(&mut header);
            let command = header.get_u8();
            self.body = Some((command, usize::from(header.get_u16())));
        }
        let Some((command, length)) = self.body else {
            return Ok(None);
        };
        if input.len() < length + MAC_LEN {
            return Ok(None);
        }
        let mut payload = input.split_to(length);
        self.rx.decrypt(&mut payload);
        self.rx.check_mac(&input.split_to(MAC_LEN))?;
        self.body = None;
        Ok(Some((command, payload.freeze())))
    }
}

fn framed(prefix: &[u8], payload: &[u8]) -> Result<Vec<u8>> {
    let length = prefix.len() + 4 + payload.len();
    let header = u32::try_from(length)?.to_be_bytes();
    let mut result = Vec::with_capacity(length);
    result.extend_from_slice(prefix);
    result.extend_from_slice(&header);
    result.extend_from_slice(payload);
    Ok(result)
}

fn platform() -> Platform {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64" | "aarch64") => Platform::WindowsX86_64,
        ("windows", _) => Platform::WindowsX86,
        ("macos", "x86_64") => Platform::MacosX86_64,
        ("macos", _) => Platform::MacosX86,
        (_, "x86_64") => Platform::LinuxX86_64,
        (_, "arm" | "aarch64") => Platform::LinuxArm,
        _ => Platform::LinuxX86,
    }
}
