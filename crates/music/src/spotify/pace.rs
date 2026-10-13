use std::time::{Duration, Instant};

use bytes::Bytes;
use http::Request;
use librespot_core::error::ErrorKind;
use librespot_core::{Error, Session};

/// How long a refused request waits before it asks librespot's limiter again. The limiter
/// frees a slot every tenth of a second, so this stays short.
const WAIT: Duration = Duration::from_millis(500);
/// How long a request keeps waiting for room before it gives up with the limiter's error.
const PATIENCE: Duration = Duration::from_secs(60);
/// How librespot's limiter words a refusal, in `HttpClient::request_fut`.
const LIMITER: &str = "rate limited for at least another";

/// Runs a request to Spotify, waiting while librespot's own rate limiter refuses it.
/// librespot allows 300 requests to spotify.com every 30 seconds and fails the rest at once
/// with `ResourceExhausted`, so a burst such as reading every playlist would otherwise cost
/// the requests made beside it, such as the liked songs. A refused request never left the
/// machine, so `request` may be a write.
pub(crate) async fn paced<T, F, R>(mut request: F) -> Result<T, Error>
where
    F: FnMut() -> R,
    R: Future<Output = Result<T, Error>>,
{
    let started = Instant::now();
    loop {
        match request().await {
            Err(error) if refused(&error) && started.elapsed() < PATIENCE => {
                tokio::time::sleep(WAIT).await;
            }
            done => return done,
        }
    }
}

/// Sends a request built by hand through librespot's HTTP client, pacing it as `paced` does.
pub(crate) async fn send(session: &Session, request: Request<Bytes>) -> Result<Bytes, Error> {
    let (parts, body) = request.into_parts();
    paced(|| {
        let mut request = Request::new(body.clone());
        *request.method_mut() = parts.method.clone();
        *request.uri_mut() = parts.uri.clone();
        *request.version_mut() = parts.version;
        *request.headers_mut() = parts.headers.clone();
        session.http_client().request_body(request)
    })
    .await
}

/// Whether librespot's own limiter turned the request down before sending it. A 429 from
/// Spotify carries the same kind, so the message tells the two apart: Spotify's answer must not
/// be asked again twice a second.
fn refused(error: &Error) -> bool {
    error.kind == ErrorKind::ResourceExhausted && error.error.to_string().starts_with(LIMITER)
}
