use gpui::{App, Context, Entity, Task};
use state::{AppSettings, Playback, QueuePosition, Reloaded, Sonora};
use ui::{Appearance, CoverPalette, Look, ThemeOverrides};

/// Drives the theme's tint from the playing track's cover, and keeps the next
/// track's cover sampled ahead of time so a track change recolours on the frame
/// it happens rather than once a download and a decode have come back.
pub struct Adaptive {
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    cover: Option<String>,
    /// The palette the theme is wearing, so a change that is not a track change can be put
    /// back on without sampling the cover again.
    painted: CoverPalette,
    /// The requested palette, kept so unrelated settings do not restart a running fade.
    applied: Option<(Look, ThemeOverrides, f32, CoverPalette)>,
    ahead: Option<Ahead>,
    task: Option<Task<()>>,
}

/// The cover of the track queued after the current one, with the palette once
/// the sampling has finished. The task is held so that requeueing cancels a
/// sampling that is no longer wanted; it is never cleared from inside itself.
struct Ahead {
    cover: String,
    palette: Option<CoverPalette>,
    _task: Task<()>,
}

impl Adaptive {
    pub fn new(playback: Entity<Playback>, cx: &mut Context<Self>) -> Self {
        let sonora = Sonora::global(cx);
        let (queue, settings) = (sonora.queue.clone(), sonora.settings.clone());
        let devices = sonora.devices.clone();
        cx.observe(&playback, |this, _, cx| this.sync(false, cx))
            .detach();
        cx.observe(&settings, |this, _, cx| {
            let settings = this.settings.read(cx);
            let instant = this
                .applied
                .as_ref()
                .is_some_and(|(look, colors, strength, _)| {
                    look.kind == settings.resolved_appearance()
                        && (*strength > 0.) == settings.ambient()
                        && *colors == settings.theme_overrides()
                });
            this.sync(instant, cx);
        })
        .detach();
        cx.observe(&devices, |this, _, cx| this.sync(false, cx))
            .detach();
        // A reload can change what the theme is built from without changing its kind or tint,
        // so forget what was worn and paint it again.
        cx.subscribe(&settings, |this, _, _: &Reloaded, cx| {
            this.applied = None;
            this.sync(false, cx);
        })
        .detach();
        cx.observe(&queue, |this, _, cx| this.look_ahead(cx))
            .detach();

        let mut adaptive = Self {
            playback,
            settings,
            cover: None,
            painted: CoverPalette::default(),
            applied: None,
            ahead: None,
            task: None,
        };
        adaptive.sync(false, cx);
        adaptive
    }

    /// Resamples the cover and hands the palette to the theme. `instant` skips the fade, for
    /// a change that is not a track change.
    fn sync(&mut self, instant: bool, cx: &mut Context<Self>) {
        let cover = self
            .settings
            .read(cx)
            .ambient()
            .then(|| {
                self.playback
                    .read(cx)
                    .shown_track(cx)
                    .and_then(|track| track.cover)
            })
            .flatten();

        // Settings also change during playback bookkeeping; a new cover always fades.
        let instant = instant && cover == self.cover;
        if cover == self.cover {
            // A settings change can alter the theme without changing the sampled cover.
            self.apply(self.painted, instant, cx);
            self.look_ahead(cx);
            return;
        }
        self.cover = cover.clone();

        let Some(cover) = cover else {
            self.task = None;
            self.apply(CoverPalette::default(), instant, cx);
            self.look_ahead(cx);
            return;
        };

        // The track we sampled ahead is usually the one that just started, so
        // the recolour lands without a round trip.
        let ready = self
            .ahead
            .take_if(|ahead| ahead.cover == cover && ahead.palette.is_some())
            .and_then(|ahead| ahead.palette);
        match ready {
            Some(palette) => {
                self.task = None;
                self.apply(palette, instant, cx);
            }
            None => {
                let palette = ui::palette(cover, cx);
                self.task = Some(cx.spawn(async move |this, cx| {
                    let palette = palette.await;
                    this.update(cx, |this, cx| this.apply(palette, instant, cx))
                        .ok();
                }));
            }
        }
        self.look_ahead(cx);
    }

    /// Puts the palette and the kind on the theme, as a fade or as a swap on the spot.
    fn apply(&mut self, palette: CoverPalette, instant: bool, cx: &mut App) {
        self.painted = palette;
        let settings = self.settings.read(cx);
        let target = (
            settings.look(),
            settings.theme_overrides(),
            settings.ambient_strength(),
            palette,
        );
        if self.applied.as_ref() == Some(&target) {
            return;
        }
        let appearance = Appearance::resolve(target.0, &target.1, palette, target.2);
        self.applied = Some(target);
        match instant {
            true => appearance.set(cx),
            false => appearance.fade(cx),
        }
    }

    /// Samples the cover of the track queued next, so its palette is waiting
    /// when it starts. A shuffle or a jump elsewhere in the library simply
    /// misses and falls back to sampling on the change.
    fn look_ahead(&mut self, cx: &mut Context<Self>) {
        let next = match self.settings.read(cx).ambient() {
            true => self
                .playback
                .read(cx)
                .shown_queue(cx)
                .track(QueuePosition::Upcoming(0), cx)
                .and_then(|track| track.cover),
            false => None,
        };
        let Some(cover) = next else {
            self.ahead = None;
            return;
        };
        if self
            .ahead
            .as_ref()
            .is_some_and(|ahead| ahead.cover == cover)
        {
            return;
        }

        let palette = ui::palette(cover.clone(), cx);
        let wanted = cover.clone();
        let task = cx.spawn(async move |this, cx| {
            let palette = palette.await;
            this.update(cx, |this, _| {
                let Some(ahead) = this.ahead.as_mut().filter(|ahead| ahead.cover == wanted) else {
                    return;
                };
                ahead.palette = Some(palette);
            })
            .ok();
        });
        self.ahead = Some(Ahead {
            cover,
            palette: None,
            _task: task,
        });
    }
}
