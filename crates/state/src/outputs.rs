//! Which output device the sound plays on, so the equalizer can switch to the profile bound to
//! it: the sink the sound server plays Sonora on under Linux, the device cpal opened elsewhere.

#[cfg(target_os = "linux")]
use std::time::Duration;

use gpui::{Context, Entity, Task};
use music::output::Device;

use crate::Playback;

/// How long the first reconnect to the sound server waits, doubling up to `RETRY_MOST`. It goes
/// away on a restart and may not be up yet when Sonora starts with the session.
#[cfg(target_os = "linux")]
const RETRY_FIRST: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
const RETRY_MOST: Duration = Duration::from_secs(60);

/// The output device the sound plays on.
pub struct Outputs {
    playback: Entity<Playback>,
    device: Option<Device>,
    _follow: Task<()>,
}

impl Outputs {
    pub fn new(playback: Entity<Playback>, cx: &mut Context<Self>) -> Self {
        Self {
            playback,
            device: None,
            _follow: follow(cx),
        }
    }

    /// The device the sound plays on, `None` while nothing says.
    pub fn device(&self) -> Option<&Device> {
        self.device.as_ref()
    }

    /// Takes in the device now played on and hands it to `Playback`, which switches the
    /// equalizer to its profile when it is another device than the last.
    fn arrive(&mut self, device: Device, cx: &mut Context<Self>) {
        self.playback
            .update(cx, |playback, cx| playback.follow_output(&device.id, cx));
        if self.device.as_ref() != Some(&device) {
            log::info!("outputs: playing on {} ({})", device.name, device.id);
            self.device = Some(device);
            cx.notify();
        }
    }

    /// Forgets the device shown, for a sound server that went away. `Playback` keeps its id, so
    /// coming back to the same device is not a move.
    #[cfg(target_os = "linux")]
    fn lose(&mut self, cx: &mut Context<Self>) {
        if self.device.take().is_some() {
            cx.notify();
        }
    }
}

/// Follows the sound server's default sink on tokio, reconnecting with a growing pause whenever
/// the server is not there or hangs up.
#[cfg(target_os = "linux")]
fn follow(cx: &mut Context<Outputs>) -> Task<()> {
    let io = crate::Io::global(cx);
    cx.spawn(async move |this, cx| {
        let mut pause = RETRY_FIRST;
        loop {
            let (sender, mut devices) = tokio::sync::mpsc::unbounded_channel();
            let server = io.spawn(music::output::follow(move |device| {
                sender.send(device).ok();
            }));
            while let Some(device) = devices.recv().await {
                pause = RETRY_FIRST;
                if this.update(cx, |this, cx| this.arrive(device, cx)).is_err() {
                    return;
                }
            }
            if let Ok(Err(error)) = server.await {
                log::debug!("outputs: the sound server is not answering: {error:#}");
            }
            if this.update(cx, |this, cx| this.lose(cx)).is_err() {
                return;
            }
            cx.background_executor().timer(pause).await;
            pause = (pause * 2).min(RETRY_MOST);
        }
    })
}

/// Follows the device cpal opens, which on macOS and Windows is the system's default output.
#[cfg(not(target_os = "linux"))]
fn follow(cx: &mut Context<Outputs>) -> Task<()> {
    let mut streams = music::output::streams();
    cx.spawn(async move |this, cx| {
        loop {
            let stream = streams.borrow_and_update().clone();
            if let Some(stream) = stream
                && this
                    .update(cx, |this, cx| this.arrive(stream.device, cx))
                    .is_err()
            {
                return;
            }
            if streams.changed().await.is_err() {
                return;
            }
        }
    })
}
