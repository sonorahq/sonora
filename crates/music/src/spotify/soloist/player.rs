//! Runs the paired Soloist daemon. Spotify Connect handles playback commands and state.

use std::ffi::OsStr;

use tokio::sync::oneshot;

use super::daemon::{RUNNING, follow, launch};
use super::{binary, data, key, prefs, recheck};
use crate::{PlaybackConfig, PlaybackEvent, PlaybackEvents};

/// Starts Soloist and stops it when the engine is dropped.
pub(crate) fn start(config: &PlaybackConfig) -> (oneshot::Sender<()>, Box<dyn PlaybackEvents>) {
    let (stop, stopped) = oneshot::channel();
    let (failure, failed) = oneshot::channel();
    let normalisation = config.normalisation;
    let gapless = config.gapless;
    let quality = config.quality;
    let volume = (config.gain.clamp(0.0, 1.0).cbrt() * 100.0)
        .round()
        .to_string();
    let thread = std::thread::Builder::new()
        .name("soloist".into())
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map(|runtime| {
                    runtime.block_on(async move {
                        let mut stopped = stopped;
                        let _running = tokio::select! {
                            running = RUNNING.lock() => running,
                            _ = &mut stopped => return,
                        };
                        let Some(key) = key() else {
                            recheck();
                            failure.send(PlaybackEvent::Refused).ok();
                            return;
                        };
                        let data = data();
                        if let Err(error) = prefs::write(&data, normalisation, gapless, quality) {
                            log::warn!("soloist: cannot write the audio settings: {error:#}");
                        }
                        let flags =
                            ["--cache-size", "512", "--initial-volume", &volume].map(OsStr::new);
                        for refresh in [true, false] {
                            let mut child = match launch(&key, &data, &flags) {
                                Ok(child) => child,
                                Err(error) => {
                                    log::warn!("soloist: {error:#}");
                                    break;
                                }
                            };
                            let mut lines = follow(&mut child, &key);
                            let status = loop {
                                tokio::select! {
                                    status = child.wait() => break status,
                                    _ = &mut stopped => {
                                        super::daemon::stop(&mut child).await;
                                        return;
                                    }
                                    Some(line) = lines.recv() => {
                                        if line.contains("login failed") {
                                            super::daemon::stop(&mut child).await;
                                            failure.send(PlaybackEvent::Refused).ok();
                                            return;
                                        }
                                    }
                                }
                            };
                            let code = status.ok().and_then(|status| status.code());
                            if !refresh || code != Some(binary::EXPIRED) {
                                log::warn!("soloist: the daemon stopped with code {code:?}");
                                break;
                            }
                            tokio::select! {
                                _ = &mut stopped => return,
                                result = binary::fetch() => if let Err(error) = result {
                                    log::warn!("soloist: cannot update the daemon: {error:#}");
                                    break;
                                }
                            }
                        }
                        failure.send(PlaybackEvent::Refused).ok();
                    });
                });
            if let Err(error) = result {
                log::error!("soloist: cannot build its runtime: {error}");
            }
        });
    if let Err(error) = thread {
        log::error!("soloist: cannot start its thread: {error}");
    }
    (stop, Box::new(Events(Some(failed))))
}

struct Events(Option<oneshot::Receiver<PlaybackEvent>>);

#[async_trait::async_trait]
impl PlaybackEvents for Events {
    async fn next(&mut self) -> Option<PlaybackEvent> {
        self.0.take()?.await.ok()
    }
}
