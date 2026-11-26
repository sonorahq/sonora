//! Runs the paired Soloist daemon. Spotify Connect handles playback commands and state.

use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use super::daemon::{Daemon, RUNNING, launch};
use super::{binary, data, key, prefs, recheck};
use music::{PlaybackConfig, PlaybackEvent, PlaybackEvents};

/// Starts Soloist and stops it when the engine is dropped.
pub fn start(
    username: &str,
    config: &PlaybackConfig,
) -> (oneshot::Sender<()>, Box<dyn PlaybackEvents>) {
    let (stop, stopped) = oneshot::channel();
    let (failure, failed) = oneshot::channel();
    let username = username.to_owned();
    let normalisation = config.normalisation;
    let gapless = config.gapless;
    let quality = config.quality;
    let volume = (config.gain.clamp(0.0, 1.0).cbrt() * 100.0)
        .round()
        .to_string();
    let thread = std::thread::Builder::new()
        .name("soloist".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => return log::error!("soloist: cannot build its runtime: {error}"),
            };
            runtime.block_on(async move {
                let mut stopped = stopped;
                let _running = tokio::select! {
                    biased;
                    _ = &mut stopped => return,
                    running = RUNNING.lock() => running,
                };
                let Some(key) = key().filter(|_| super::set_up_for(&username)) else {
                    recheck();
                    failure.send(PlaybackEvent::Refused).ok();
                    return;
                };
                let data = data();
                if let Err(error) = prefs::write(&data, normalisation, gapless, quality) {
                    log::warn!("soloist: cannot write the audio settings: {error:#}");
                }
                let flags = ["--cache-size", "512", "--initial-volume", &volume].map(OsStr::new);
                let mut child = None;
                tokio::select! {
                    biased;
                    _ = &mut stopped => {}
                    () = supervise(&key, &data, &flags, &mut child) => {
                        failure.send(PlaybackEvent::Refused).ok();
                    }
                }
                if let Some(mut child) = child {
                    child.stop().await;
                }
            });
        });
    if let Err(error) = thread {
        log::error!("soloist: cannot start its thread: {error}");
    }
    (stop, Box::new(Events(Some(failed))))
}

/// Retries recoverable failures. Only consecutive login refusals end supervision.
async fn supervise(api_key: &str, data: &Path, flags: &[&OsStr], child: &mut Option<Daemon>) {
    const RETRY_FIRST: Duration = Duration::from_secs(5);
    const RETRY_LAST: Duration = Duration::from_secs(60);

    let mut delay = RETRY_FIRST;
    let mut refreshed = false;
    let mut refused_before = false;
    loop {
        let started = Instant::now();
        let exit = match launch(api_key, data, flags) {
            Ok(daemon) => child
                .insert(daemon)
                .wait(|_| {})
                .await
                .map_err(anyhow::Error::from),
            Err(error) => Err(error),
        };
        if started.elapsed() >= RETRY_LAST {
            delay = RETRY_FIRST;
            refreshed = false;
            refused_before = false;
        }
        let refused = matches!(exit, Ok(None));
        if refused && refused_before {
            return;
        }
        refused_before = refused;

        if let Some(daemon) = child.as_mut()
            && !matches!(exit, Ok(Some(_)))
        {
            daemon.stop().await;
        }
        *child = None;
        match exit {
            Ok(Some(status)) if status.code() == Some(binary::EXPIRED) && !refreshed => {
                match binary::fetch().await {
                    Ok(_) => {
                        refreshed = true;
                        continue;
                    }
                    Err(error) => log::warn!("soloist: cannot update the daemon: {error:#}"),
                }
            }
            Ok(Some(status)) => log::warn!("soloist: the daemon stopped: {status}"),
            Ok(None) => {}
            Err(error) => log::warn!("soloist: {error:#}"),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(RETRY_LAST);
    }
}

struct Events(Option<oneshot::Receiver<PlaybackEvent>>);

#[async_trait::async_trait]
impl PlaybackEvents for Events {
    async fn next(&mut self) -> Option<PlaybackEvent> {
        self.0.take()?.await.ok()
    }
}
