//! What this device tells the server it is playing, the way every Jellyfin client does.
//!
//! The server shows it as the device's now playing on its dashboard, and counts a song as
//! played when its report stops. A session that sends nothing for five minutes is taken for
//! gone and stopped by the server, so a beat repeats the last report while a track is on.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Result;
use music::Report;
use reqwest::Method;
use tokio::task::AbortHandle;

use crate::wire;

/// How often the beat repeats the report, well inside the server's five minutes.
const BEAT: Duration = Duration::from_secs(30);
/// How many failed beats in a row end the beat, three of them still well inside five minutes.
const MISSES: u32 = 3;

/// The track this device last said it plays, where it was and since when.
struct Now {
    track: String,
    position: Duration,
    since: Instant,
    paused: bool,
    beat: AbortHandle,
}

impl Now {
    /// Where the track is by now, counting the time it has played since the last report.
    fn position(&self) -> Duration {
        match self.paused {
            true => self.position,
            false => self.position + self.since.elapsed(),
        }
    }
}

/// The reports one account sends. Clones share the track on, so a report from any copy of the
/// client moves the same session.
#[derive(Clone, Default)]
pub struct Session {
    now: Arc<Mutex<Option<Now>>>,
}

/// Sends one report, as `JellyfinClient` does for its own requests.
pub type Sender = Arc<
    dyn Fn(
            Method,
            &'static str,
            serde_json::Value,
        ) -> futures::future::BoxFuture<'static, Result<()>>
        + Send
        + Sync,
>;

impl Session {
    /// Reports `track` at `position`. The first report of a track starts its session on the
    /// server, a stop ends it, and the beat runs in between.
    pub async fn report(
        &self,
        send: Sender,
        track: &str,
        report: Report,
        position: Duration,
    ) -> Result<()> {
        let paused = report == Report::Paused;
        let started = {
            let now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
            now.as_ref().is_some_and(|now| now.track == track)
        };
        if report == Report::Stopped {
            if let Some(now) = self.take(track) {
                now.beat.abort();
            }
            return send(
                Method::POST,
                "/Sessions/Playing/Stopped",
                body(track, position, false),
            )
            .await;
        }
        if !started {
            if let Some(previous) = self
                .now
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
            {
                previous.beat.abort();
            }
            send(
                Method::POST,
                "/Sessions/Playing",
                body(track, position, paused),
            )
            .await?;
            let beat = self.beat(send.clone());
            *self.now.lock().unwrap_or_else(PoisonError::into_inner) = Some(Now {
                track: track.to_owned(),
                position,
                since: Instant::now(),
                paused,
                beat,
            });
        } else if let Some(now) = self
            .now
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            now.position = position;
            now.since = Instant::now();
            now.paused = paused;
        }
        send(
            Method::POST,
            "/Sessions/Playing/Progress",
            body(track, position, paused),
        )
        .await
    }

    /// The track on, taken off when it is `track`.
    fn take(&self, track: &str) -> Option<Now> {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        match now.as_ref().is_some_and(|now| now.track == track) {
            true => now.take(),
            false => None,
        }
    }

    /// Repeats the report every [`BEAT`] until the track stops. After [`MISSES`] failed reports
    /// in a row, as once the account signs out, it forgets the track, so the next report starts
    /// the session over with a beat of its own.
    fn beat(&self, send: Sender) -> AbortHandle {
        let now = self.now.clone();
        tokio::spawn(async move {
            let mut missed = 0;
            loop {
                tokio::time::sleep(BEAT).await;
                let report = {
                    let now = now.lock().unwrap_or_else(PoisonError::into_inner);
                    now.as_ref()
                        .map(|now| body(&now.track, now.position(), now.paused))
                };
                let Some(report) = report else {
                    return;
                };
                match send(Method::POST, "/Sessions/Playing/Progress", report).await {
                    Ok(()) => missed = 0,
                    Err(error) => {
                        missed += 1;
                        log::info!("jellyfin: a session beat failed: {error:#}");
                        if missed >= MISSES {
                            now.lock().unwrap_or_else(PoisonError::into_inner).take();
                            return;
                        }
                    }
                }
            }
        })
        .abort_handle()
    }
}

fn body(track: &str, position: Duration, paused: bool) -> serde_json::Value {
    serde_json::json!({
        "ItemId": track,
        "PositionTicks": wire::to_ticks(position),
        "IsPaused": paused,
        "CanSeek": true,
        "PlayMethod": "DirectPlay",
    })
}
