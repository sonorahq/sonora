use gpui::{App, Context, Entity, Task};
use state::{AppSettings, Playback, QueuePosition, Reloaded, Sonora};
use ui::{ActiveTheme as _, CoverPalette, Look, Theme, ThemeKind, ThemeOverrides};

/// Drives the theme's tint from the playing track's cover, and keeps the next
/// track's cover sampled ahead of time so a track change recolours on the frame
/// it happens rather than once a download and a decode have come back.
pub struct Adaptive {
    playback: Entity<Playback>,
    settings: Entity<AppSettings>,
    /// Whether the fullscreen shell is up. The ambient background is painted
    /// out of the tint, so fullscreen samples the cover whatever the adaptive
    /// theme setting says.
    fullscreen: bool,
    cover: Option<String>,
    /// The palette the theme is wearing, so a change that is not a track change can be put
    /// back on without sampling the cover again.
    painted: CoverPalette,
    /// The kind last applied from here and whether the theme file's colours were left off,
    /// since the theme itself carries neither.
    worn: Option<(ThemeKind, bool)>,
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
        cx.observe(&playback, |this, _, cx| this.sync(false, cx))
            .detach();
        cx.observe(&settings, |this, _, cx| this.sync(false, cx))
            .detach();
        // A reload can change what the theme is built from without changing its kind or tint,
        // so forget what was worn and paint it again.
        cx.subscribe(&settings, |this, _, _: &Reloaded, cx| {
            this.worn = None;
            this.sync(false, cx);
        })
        .detach();
        cx.observe(&queue, |this, _, cx| this.look_ahead(cx))
            .detach();

        let mut adaptive = Self {
            playback,
            settings,
            fullscreen: false,
            cover: None,
            painted: CoverPalette::default(),
            worn: None,
            ahead: None,
            task: None,
        };
        adaptive.sync(false, cx);
        adaptive
    }

    /// Records whether fullscreen is up and resamples. Leaving fullscreen with the adaptive
    /// theme off drops the tint here, so the rest of the app never keeps a cover's colour.
    /// The swap is instant either way: the whole palette appears or disappears at once, and
    /// washing the workspace through that on the way out is a change nobody asked for.
    pub fn set_fullscreen(&mut self, fullscreen: bool, cx: &mut Context<Self>) {
        if self.fullscreen == fullscreen {
            return;
        }
        self.fullscreen = fullscreen;
        self.sync(true, cx);
    }

    /// Resamples the cover and hands the palette to the theme. `instant` skips the fade, for
    /// a change that is not a track change.
    fn sync(&mut self, instant: bool, cx: &mut Context<Self>) {
        let cover = self
            .settings
            .read(cx)
            .cover_tint(self.fullscreen)
            .then(|| {
                self.playback
                    .read(cx)
                    .shown_track(cx)
                    .and_then(|track| track.cover)
            })
            .flatten();

        if cover == self.cover {
            // The kind can still have changed under the same cover, which is what entering
            // and leaving fullscreen does.
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
        match self.ready(&cover) {
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

    /// Whether the ambient background is up, which puts the plain adaptive Dark theme on. The
    /// field behind the lyrics is dark whatever the cover gave it, so a light theme or a theme
    /// file's own colours would lose their text on it.
    fn plain(&self, cx: &App) -> bool {
        self.fullscreen && self.settings.read(cx).ambient()
    }

    /// The kind and overrides to wear. Under the ambient background only the theme file's
    /// radius and font size survive, so the layout does not shift on the way in.
    fn dress(&self, cx: &App) -> (ThemeKind, ThemeOverrides) {
        let settings = self.settings.read(cx);
        let overrides = settings.theme_overrides();
        match self.plain(cx) {
            true => (ThemeKind::Dark, stripped(overrides)),
            false => (settings.look().kind, overrides),
        }
    }

    /// Puts the palette and the dress on the theme, as a fade or as a swap on the spot.
    fn apply(&mut self, palette: CoverPalette, instant: bool, cx: &mut App) {
        let plain = self.plain(cx);
        let (kind, overrides) = self.dress(cx);
        let theme = *cx.theme();
        if self.worn == Some((kind, plain))
            && theme.tint == palette.primary
            && theme.tint_secondary == palette.secondary
        {
            return;
        }
        self.worn = Some((kind, plain));
        self.painted = palette;

        let look = Look {
            kind,
            tint: palette.primary,
            tint_secondary: palette.secondary,
            ..self.settings.read(cx).look()
        };

        match instant {
            true => Theme::set(look, &overrides, cx),
            false => Theme::fade(look, &overrides, cx),
        }
    }

    /// The palette sampled ahead for `cover`, taken out of the slot so the next
    /// track can claim it.
    fn ready(&mut self, cover: &str) -> Option<CoverPalette> {
        let palette = self
            .ahead
            .as_ref()
            .filter(|ahead| ahead.cover == cover)?
            .palette?;
        self.ahead = None;
        Some(palette)
    }

    /// Samples the cover of the track queued next, so its palette is waiting
    /// when it starts. A shuffle or a jump elsewhere in the library simply
    /// misses and falls back to sampling on the change.
    fn look_ahead(&mut self, cx: &mut Context<Self>) {
        let next = match self.settings.read(cx).cover_tint(self.fullscreen) {
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

/// The theme fullscreen wears over the ambient background for a cover's `palette`, for a preview
/// that has to look like fullscreen while the app wears the chosen theme.
pub(crate) fn stage(palette: CoverPalette, cx: &App) -> Theme {
    let settings = Sonora::global(cx).settings.read(cx);
    let look = Look {
        kind: ThemeKind::Dark,
        tint: palette.primary,
        tint_secondary: palette.secondary,
        ..settings.look()
    };
    Theme::for_look(look, &stripped(settings.theme_overrides()))
}

/// The overrides with every colour taken out, leaving the radius and font size.
fn stripped(overrides: ThemeOverrides) -> ThemeOverrides {
    ThemeOverrides {
        radius: overrides.radius,
        font_size: overrides.font_size,
        ..ThemeOverrides::default()
    }
}
