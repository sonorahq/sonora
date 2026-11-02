//! A loopback HTTP server for the SoundCloud tests: it answers every request with whatever the
//! test's route says and keeps what was sent, so a test can check both sides of an exchange
//! without reaching SoundCloud.

use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// How a test answers one request line: a status and a json body.
type Route = Arc<dyn Fn(&str) -> (u16, String) + Send + Sync>;

/// One request as the server read it.
#[derive(Clone, Debug)]
pub(crate) struct Sent {
    /// The request line, such as `GET /me?client_id=x HTTP/1.1`.
    pub line: String,
    /// The header block, one `name: value` per line.
    pub headers: String,
    pub body: String,
}

pub(crate) struct Mock {
    base: String,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Mock {
    /// Starts a server on a free loopback port. It lives until the test's runtime ends.
    pub async fn start(route: impl Fn(&str) -> (u16, String) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let route: Route = Arc::new(route);
        let kept = sent.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let Some(request) = read(&mut stream).await else {
                    continue;
                };
                let (status, body) = route(&request.line);
                kept.lock().unwrap().push(request);
                let answer = format!(
                    "HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(answer.as_bytes()).await.ok();
                stream.shutdown().await.ok();
            }
        });
        Self { base, sent }
    }

    /// The server's own address with `path` on the end.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Everything sent so far, oldest first.
    pub fn requests(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }

    /// An http client that reaches the loopback server directly, whatever proxy the
    /// environment names.
    pub fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }
}

/// Reads one request: the head up to the blank line, then as much body as it announces.
async fn read(stream: &mut tokio::net::TcpStream) -> Option<Sent> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let (line, headers) = head.split_once("\r\n").unwrap_or((head.as_str(), ""));
    let length = headers
        .lines()
        .find_map(|header| {
            let (name, value) = header.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Some(Sent {
        line: line.to_owned(),
        headers: headers.to_owned(),
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}
