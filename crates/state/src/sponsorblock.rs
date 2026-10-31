use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use moka::sync::Cache;
use music::{SkipSegment, SponsorBlock as Client};

use crate::playback::PlaybackEvent;
use crate::{AppSettings, Io, Network, Playback, PlaybackState, Session, SessionEvent, join};

const CACHE_CAPACITY: u64 = 32;
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

/// Follows local YouTube playback and seeks over sections marked by SponsorBlock.
pub struct SponsorBlock {
    playback: Entity<Playback>,
    session: Entity<Session>,
    settings: Entity<AppSettings>,
    client: Client,
    video: Option<String>,
    segments: Vec<SkipSegment>,
    cache: Cache<String, Vec<SkipSegment>>,
    /// The last attempted skip, until the engine confirms it landed outside that section.
    skipped: Option<usize>,
    lookup: Option<Task<()>>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl SponsorBlock {
    pub fn new(
        playback: Entity<Playback>,
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        network: &Entity<Network>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&playback, |this, _, cx| this.follow(cx))
            .detach();
        cx.observe(&settings, |this, _, cx| this.follow(cx))
            .detach();
        cx.observe(&session, |this, _, cx| this.follow(cx)).detach();
        cx.observe(network, |this, _, cx| this.follow(cx)).detach();
        cx.subscribe(&session, |this, _, event, cx| {
            if matches!(event, SessionEvent::SignedOut) {
                this.cache.invalidate_all();
            }
            if matches!(event, SessionEvent::SignedOut | SessionEvent::WentOffline) {
                this.clear();
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&playback, |this, playback, event, cx| match event {
            PlaybackEvent::EndedPlayback => {
                this.clear();
                cx.notify();
            }
            PlaybackEvent::Seeked => {
                let at = playback.read(cx).position();
                if let Some(segment) = this.skipped.and_then(|index| this.segments.get(index))
                    && (at < segment.start || at >= segment.end)
                {
                    this.skipped = None;
                }
            }
            _ => {}
        })
        .detach();
        Self {
            playback,
            session,
            settings,
            client: Client::default(),
            video: None,
            segments: Vec::new(),
            cache: Cache::builder()
                .max_capacity(CACHE_CAPACITY)
                .time_to_live(CACHE_TTL)
                .build(),
            skipped: None,
            lookup: None,
            cancel: None,
        }
    }

    fn clear(&mut self) {
        self.cancel = None;
        self.lookup = None;
        self.video = None;
        self.segments.clear();
        self.skipped = None;
    }

    /// The current local track's skip ranges, merged and normalized for a seek bar.
    pub fn ranges(&self, cx: &App) -> Vec<(f32, f32)> {
        let playback = self.playback.read(cx);
        let Some(track) = playback.track() else {
            return Vec::new();
        };
        let duration = track.duration;
        if duration.is_zero()
            || !self.settings.read(cx).sponsorblock()
            || Network::lost(cx)
            || playback.steered(cx).is_some()
            || track.id.as_deref() != self.video.as_deref()
            || track
                .id
                .as_deref()
                .and_then(|id| self.session.read(cx).slug_for(id))
                != Some("youtube")
        {
            return Vec::new();
        }
        let mut ranges: Vec<(f32, f32)> = Vec::new();
        for segment in self
            .segments
            .iter()
            .filter(|segment| segment.matches_duration(duration))
        {
            let start = (segment.start.as_secs_f64() / duration.as_secs_f64()).clamp(0., 1.) as f32;
            let end = (segment.end.as_secs_f64() / duration.as_secs_f64()).clamp(0., 1.) as f32;
            if end <= start {
                continue;
            }
            if let Some(last) = ranges.last_mut()
                && start <= last.1
            {
                last.1 = last.1.max(end);
            } else {
                ranges.push((start, end));
            }
        }
        ranges
    }

    /// A lookup never holds up playback; changing tracks or disabling the setting drops it.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let video = self.playback.read(cx).track().and_then(|track| {
            let id = track.id.as_deref()?;
            (self.settings.read(cx).sponsorblock()
                && !Network::lost(cx)
                && self.session.read(cx).slug_for(id) == Some("youtube"))
            .then(|| id.to_owned())
        });
        if video != self.video {
            self.clear();
            self.video = video;
            cx.notify();
            if let Some(video) = self.video.clone() {
                if let Some(segments) = self.cache.get(&video) {
                    self.segments = segments;
                    self.skip(cx);
                    return;
                }
                let client = self.client.clone();
                let io = Io::global(cx);
                let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
                self.cancel = Some(cancel);
                self.lookup = Some(cx.spawn(async move |this, cx| {
                    let requested = video.clone();
                    let found = join(io.spawn(async move {
                        tokio::select! {
                            found = client.segments(&requested) => found,
                            _ = cancelled => Ok(Vec::new()),
                        }
                    }))
                    .await;
                    this.update(cx, |this, cx| {
                        if this.video.as_deref() != Some(video.as_str()) {
                            return;
                        }
                        match found {
                            Ok(segments) => {
                                this.cache.insert(video.clone(), segments.clone());
                                this.segments = segments;
                            }
                            Err(error) => log::warn!("sponsorblock: {error:#}"),
                        }
                        this.follow(cx);
                        cx.notify();
                    })
                    .ok();
                }));
            }
        }
        self.skip(cx);
    }

    /// Retains sections for replay, but attempts each skip once until its seek lands.
    fn skip(&mut self, cx: &mut Context<Self>) {
        let playback = self.playback.read(cx);
        if playback.state() != &PlaybackState::Playing {
            return;
        }
        let at = playback.live_position();
        let duration = playback
            .track()
            .map(|track| track.duration)
            .unwrap_or_default();
        let Some(index) = self.segments.iter().position(|segment| {
            segment.matches_duration(duration) && at >= segment.start && at < segment.end
        }) else {
            return;
        };
        if self.skipped == Some(index) {
            return;
        }
        let segment = self.segments[index];
        // Join only sections matching this recording, even when categories overlap.
        let mut end = segment.end;
        for next in &self.segments[index + 1..] {
            if next.start > end {
                break;
            }
            if next.matches_duration(duration) {
                end = end.max(next.end);
            }
        }
        let end = end.min(duration);
        if end > at {
            self.skipped = Some(index);
            self.playback.update(cx, |playback, cx| {
                playback.locally(cx, |playback, cx| playback.seek(end, cx));
            });
        }
    }
}
