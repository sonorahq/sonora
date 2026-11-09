//! The head of a page that shows one collection: a big cover, the kind of page above a large
//! title, a line of facts under it and a row of actions (Play, Shuffle, Like, More), with the
//! Play and Shuffle buttons that start the page's tracks through the store.

use std::rc::Rc;

use i18n::t;
use music::{Album, Track};
use rune::{El, PointerPress, Transition, Tx, col, icon, row, text};
use ui::{
    Button, Cover, ExplicitBadge, MenuItem, Perch, Skeleton, Text, Theme, Tipped as _, Weight,
    font, upper,
};

use crate::Ui;
use crate::shared::page::rems;
use crate::shared::tracks::TrackRows;
use state::Origin;

/// How wide the skeleton bars of a pending hero reach, for the title, the meta line and the
/// row of actions.
const TITLE_SKELETON: f32 = 240.;
const META_SKELETON: f32 = 180.;
const ACTION_SKELETON: f32 = 96.;
/// The space between the cover and the text beside it, in rems, as are the spaces below.
const COVER_GAP: f32 = 1.25;
/// The space between the lines beside the cover.
const LINE_GAP: f32 = 0.5;
/// The space between the title and the explicit badge after it.
const TITLE_GAP: f32 = 0.75;
/// The space between the facts of the meta line and the bullets between them.
const META_GAP: f32 = 0.25;
/// How far the actions sit below the meta line, on top of the line gap.
const ACTIONS_DROP: f32 = 0.25;
/// The room under the hero, above the table header.
const BOTTOM: f32 = 1.5;
/// The cover's corners against the theme's radius.
const COVER_ROUNDING: f32 = 1.5;
/// The space between the buttons in a row of hero actions, in rems.
pub(crate) const ACTION_GAP: f32 = 0.5;
/// A button's icon side, for the Play button's own glyph.
const GLYPH: f32 = 16.;
/// How small a swapped glyph starts and ends, as it pops.
const POP: f32 = 0.6;

/// A handler a hero runs with the pointer's place, such as opening a menu on a right-click.
type OnContext = Rc<dyn Fn(&mut Ui, PointerPress, &mut Tx)>;

/// One copyright line, with a © in front unless the provider already led it with © or ℗.
pub(crate) fn copyright_notice(text: &str) -> String {
    match text.starts_with(['©', '℗']) {
        true => text.to_owned(),
        false => t!("song-copyright", notice = text).into_owned(),
    }
}

/// The notices an album page prints under its tracks: the provider's copyright lines, or the
/// label behind ℗ when the provider names only that. Empty when it names neither.
pub(crate) fn copyright_notices(album: &Album) -> Vec<String> {
    if !album.copyrights.is_empty() {
        return album
            .copyrights
            .iter()
            .map(|line| copyright_notice(line))
            .collect();
    }
    match album.label.trim() {
        "" => Vec::new(),
        label => vec![t!("album-label", label = label).into_owned()],
    }
}

/// A `YYYY-MM-DD` date as the language writes one; anything else as given.
pub(crate) fn release_date_label(value: &str) -> String {
    let parts: Vec<_> = value.split('-').collect();
    if parts.len() != 3 {
        return value.to_owned();
    }
    let month = match parts[1] {
        "01" => t!("month-1"),
        "02" => t!("month-2"),
        "03" => t!("month-3"),
        "04" => t!("month-4"),
        "05" => t!("month-5"),
        "06" => t!("month-6"),
        "07" => t!("month-7"),
        "08" => t!("month-8"),
        "09" => t!("month-9"),
        "10" => t!("month-10"),
        "11" => t!("month-11"),
        "12" => t!("month-12"),
        _ => return value.to_owned(),
    };
    let day = parts[2].trim_start_matches('0');

    t!("date-full", month = &month, day = day, year = parts[0]).into_owned()
}

/// The space between the hero's actions.
pub(crate) fn action_gap(theme: &Theme) -> f32 {
    rems(theme, ACTION_GAP)
}

/// The line of facts under a hero title, joined by bullets. Its text items reach screen readers
/// as text, while an element item has to name itself.
#[derive(Default)]
pub(crate) struct MetaStrip {
    items: Vec<MetaItem>,
}

/// One fact on a [`MetaStrip`].
enum MetaItem {
    Text(String),
    Element(El),
}

impl MetaStrip {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn text(mut self, text: impl Into<String>) -> Self {
        self.items.push(MetaItem::Text(text.into()));
        self
    }

    /// An element among the facts, such as the artists as links.
    pub(crate) fn item(mut self, item: El) -> Self {
        self.items.push(MetaItem::Element(item));
        self
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn build(self, theme: &Theme) -> El {
        let style = font(theme, Text::Small, Weight::NORMAL);
        let muted = theme.muted_foreground.color();
        let mut strip = row()
            .key("hero-meta")
            .flex_wrap()
            .center_y()
            .min_w(0.)
            .gap(rems(theme, META_GAP));
        for (index, item) in self.items.into_iter().enumerate() {
            let item = match item {
                MetaItem::Text(value) => text(value.clone())
                    .key(format!("hero-meta-{index}"))
                    .role("text")
                    .label(value)
                    .font(style)
                    .color(muted),
                MetaItem::Element(item) => item,
            };
            let mut fact = row()
                .key(format!("fact-{index}"))
                .center_y()
                .gap(rems(theme, META_GAP));
            if index > 0 {
                fact = fact.child(text("•").font(style).color(muted).a11y_hidden());
            }
            strip = strip.child(fact.child(item));
        }
        strip
    }
}

/// The tracks a Play or Shuffle button starts: ones the page holds, a table's rows in the order
/// they show, or a collection fetched when the button is pressed.
#[derive(Clone)]
pub(crate) enum Listing {
    Owned(Rc<Vec<Track>>),
    Listed(TrackRows),
    Origin(Option<Origin>),
}

impl Listing {
    fn can_start(&self, ui: &Ui) -> bool {
        match self {
            Self::Owned(tracks) => tracks.iter().any(|track| track.playable),
            Self::Listed(rows) => rows.first_playable(ui).is_some(),
            Self::Origin(origin) => origin.is_some(),
        }
    }

    fn holds(&self, ui: &Ui, id: &str) -> bool {
        match self {
            Self::Owned(tracks) => tracks.iter().any(|track| track.id.as_deref() == Some(id)),
            Self::Listed(rows) => rows.holds(ui, id),
            Self::Origin(_) => false,
        }
    }

    fn whence(&self) -> Option<&Origin> {
        match self {
            Self::Owned(_) => None,
            Self::Listed(rows) => rows.whence(),
            Self::Origin(origin) => origin.as_ref(),
        }
    }

    /// Whether the player's current track came from here: `Some(true)` while it plays.
    fn playing(&self, ui: &Ui) -> Option<bool> {
        let playback = &ui.store.playback;
        match self.whence() {
            Some(origin) => playback.playing_from(origin, &ui.store.queue),
            None => playback
                .track()
                .and_then(|track| track.id.as_deref())
                .filter(|current| self.holds(ui, current))
                .and_then(|_| playback.control()),
        }
    }

    /// Loads a collection on demand or starts the tracks the page holds.
    fn start(&self, ui: &mut Ui, shuffle: bool) {
        match self {
            Self::Origin(origin) => {
                if let Some(origin) = origin.clone() {
                    match shuffle {
                        true => ui.store.shuffle_collection(origin),
                        false => ui.store.play_collection(origin),
                    }
                }
            }
            Self::Owned(tracks) => {
                let tracks = tracks.to_vec();
                match shuffle {
                    true => ui.store.shuffle_tracks(tracks, None),
                    false => ui.store.play_any(tracks, None),
                }
            }
            Self::Listed(rows) => return rows.start(ui, shuffle),
        }
        ui.settle();
    }
}

/// A page's Play button, which pauses and resumes while the player plays from the page, or its
/// Shuffle button beside it.
pub(crate) struct PlayButton {
    key: String,
    listing: Listing,
    shuffled: bool,
}

impl PlayButton {
    /// The Play button over `listing`.
    pub(crate) fn new(key: impl Into<String>, listing: Listing) -> Self {
        Self {
            key: key.into(),
            listing,
            shuffled: false,
        }
    }

    /// The Shuffle button beside Play: an icon-only outline button that turns shuffle on and
    /// starts the tracks from a random one, never mirroring the transport state.
    pub(crate) fn shuffle(key: impl Into<String>, listing: Listing) -> Self {
        Self {
            shuffled: true,
            ..Self::new(key, listing)
        }
    }

    /// The labels the Play button switches between. Its width fits the wider, so the buttons
    /// beside it stay put as playback starts and stops.
    fn labels() -> [String; 2] {
        [
            t!("common-play").into_owned(),
            t!("play-pause").into_owned(),
        ]
    }

    /// How wide the button draws, the same whatever its label says.
    pub(crate) fn width(&self, theme: &Theme) -> f32 {
        match self.shuffled {
            true => Button::width(std::iter::empty::<&str>(), theme),
            false => Button::width(Self::labels(), theme),
        }
    }

    /// The shuffle as a menu item, for a hero whose Shuffle button folded into its overflow
    /// menu.
    pub(crate) fn shuffle_item(self, ui: &Ui) -> MenuItem<Ui> {
        let disabled = !self.listing.can_start(ui);
        let listing = self.listing;
        let item = MenuItem::new("hero-shuffle", t!("play-shuffle").into_owned())
            .icon("icons/shuffle.svg")
            .on_press(move |ui: &mut Ui| listing.start(ui, true));
        match disabled {
            true => item.disabled(),
            false => item,
        }
    }

    pub(crate) fn build(self, ui: &Ui) -> El {
        let theme = &ui.theme;
        let disabled = !self.listing.can_start(ui);
        let listing = self.listing;
        if self.shuffled {
            return Button::new(self.key)
                .icon("icons/shuffle.svg")
                .tooltip(t!("play-shuffle"))
                .outline()
                .disabled(disabled)
                .on_press(move |ui: &mut Ui| listing.start(ui, true))
                .build(theme);
        }
        let playing = listing.playing(ui);
        let [play, pause] = Self::labels();
        let (label, glyph) = match playing {
            Some(true) => (pause.clone(), "icons/pause.svg"),
            Some(false) | None => (play.clone(), "icons/play.svg"),
        };
        let ink = match disabled {
            true => theme.muted_foreground,
            false => theme.primary_foreground,
        };
        // The glyph is keyed by what it shows, so swapping play for pause pops the new one in
        // while the old one shrinks away.
        let swapped = row()
            .key("glyph")
            .size(GLYPH, GLYPH)
            .child(
                icon(icons::get(glyph), GLYPH)
                    .key(glyph)
                    .absolute(0., 0.)
                    .color(ink.color())
                    .transition(Transition::scale(POP).allow_small_scale()),
            )
            .a11y_hidden();
        Button::new(self.key)
            .label(label)
            .reserve([play, pause])
            .leading(swapped)
            .primary()
            .disabled(disabled)
            .on_press(move |ui: &mut Ui| match playing {
                Some(_) => {
                    ui.store.toggle_play();
                    ui.settle();
                }
                None => listing.start(ui, false),
            })
            .build(theme)
    }
}

/// A hero's heart: an outline button that saves or unsaves the page's collection, filled in the
/// accent while saved. The heart pops as it fills or empties.
pub(crate) fn like_button(
    key: impl Into<String>,
    saved: bool,
    pending: bool,
    press: impl Fn(&mut Ui) + 'static,
    theme: &Theme,
) -> El {
    let glyph = match saved {
        true => "icons/heart-filled.svg",
        false => "icons/heart.svg",
    };
    let ink = match (pending, saved) {
        (true, _) => theme.muted_foreground,
        (false, true) => theme.primary,
        (false, false) => theme.foreground,
    };
    let heart = row()
        .key("heart")
        .size(GLYPH, GLYPH)
        .child(
            icon(icons::get(glyph), GLYPH)
                .key(glyph)
                .absolute(0., 0.)
                .color(ink.color())
                .transition(Transition::scale(POP).allow_small_scale()),
        )
        .a11y_hidden();
    Button::new(key)
        .outline()
        .leading(heart)
        .tooltip(match saved {
            true => t!("menu-remove-from-library"),
            false => t!("menu-add-to-library"),
        })
        .toggled(saved)
        .disabled(pending)
        .on_press(press)
        .build(theme)
}

/// The head of a collection page. Build it with [`Hero::build`]; set [`Hero::padded`] when it
/// leads a table, which spans the page while the hero keeps the page's inset.
pub(crate) struct Hero {
    key: String,
    title: String,
    cover: Option<String>,
    fallback: Option<&'static str>,
    accent: bool,
    eyebrow: Option<String>,
    meta: Option<El>,
    actions: Option<El>,
    circle: bool,
    explicit: bool,
    pending: bool,
    padded: bool,
    on_context: Option<OnContext>,
}

impl Hero {
    pub(crate) fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            cover: None,
            fallback: None,
            accent: false,
            eyebrow: None,
            meta: None,
            actions: None,
            circle: false,
            explicit: false,
            pending: false,
            padded: false,
            on_context: None,
        }
    }

    /// The width a hero `width` wide leaves beside its cover for the title and the actions.
    pub(crate) fn room(width: f32, theme: &Theme) -> f32 {
        width - theme.metrics.cover - rems(theme, COVER_GAP).floor()
    }

    pub(crate) fn cover(mut self, cover: Option<String>) -> Self {
        self.cover = cover;
        self
    }

    /// The glyph the cover shows without art, as `"icons/<name>.svg"`.
    pub(crate) fn fallback(mut self, icon: &'static str) -> Self {
        self.fallback = Some(icon);
        self
    }

    /// Draws the fallback glyph in the accent.
    pub(crate) fn accent(mut self) -> Self {
        self.accent = true;
        self
    }

    /// The kind of page, drawn in capitals above the title.
    pub(crate) fn eyebrow(mut self, eyebrow: impl Into<String>) -> Self {
        self.eyebrow = Some(eyebrow.into());
        self
    }

    /// The line of facts under the title, usually a built [`MetaStrip`].
    pub(crate) fn meta(mut self, meta: El) -> Self {
        self.meta = Some(meta);
        self
    }

    /// The row of buttons along the bottom; space them with [`action_gap`].
    pub(crate) fn actions(mut self, actions: El) -> Self {
        self.actions = Some(actions);
        self
    }

    /// A round cover, for a person or an artist.
    pub(crate) fn circle(mut self) -> Self {
        self.circle = true;
        self
    }

    pub(crate) fn explicit(mut self, explicit: bool) -> Self {
        self.explicit = explicit;
        self
    }

    /// Draws the cover, title, meta line and actions as skeletons while the page has nothing to
    /// fill them with yet. The eyebrow stays, since it names the kind of page.
    pub(crate) fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }

    /// Pads the hero by the page's inset at its sides and top.
    pub(crate) fn padded(mut self) -> Self {
        self.padded = true;
        self
    }

    /// A right-click on the cover or the title, where the page's menu opens.
    pub(crate) fn on_context(
        mut self,
        f: impl Fn(&mut Ui, PointerPress, &mut Tx) + 'static,
    ) -> Self {
        self.on_context = Some(Rc::new(f));
        self
    }

    pub(crate) fn build(self, ui: &Ui) -> El {
        let theme = &ui.theme;
        let art = theme.metrics.cover;
        let corner = match self.circle {
            true => art / 2.,
            false => theme.radius * COVER_ROUNDING,
        };
        let eyebrow = self.eyebrow.map(|label| {
            text(upper(&label))
                .key("hero-eyebrow")
                .role("text")
                .label(label)
                .font(font(theme, Text::Small, Weight::SEMIBOLD))
                .color(theme.muted_foreground.color())
        });
        let mut hero = row()
            .key(self.key.clone())
            .w_full()
            .items_end()
            .gap(rems(theme, COVER_GAP).floor())
            .pb(rems(theme, BOTTOM));
        if self.padded {
            hero = hero.px(theme.metrics.inset).pt(theme.metrics.inset);
        }
        let lines = col()
            .key("hero-lines")
            .grow(1.)
            .min_w(0.)
            .h(art)
            .justify_end()
            .gap(rems(theme, LINE_GAP))
            .children(eyebrow);

        if self.pending {
            let display = theme.text(Text::Display);
            let small = theme.text(Text::Small);
            let line =
                |size: f32, bar: El| row().h((size * ui::LEADING).round()).center_y().child(bar);
            return hero
                .child(Skeleton::new().build(theme).size(art, art).radius(corner))
                .child(
                    lines
                        .child(line(
                            display,
                            Skeleton::new().build(theme).size(TITLE_SKELETON, display),
                        ))
                        .child(line(
                            small,
                            Skeleton::new()
                                .build(theme)
                                .size(META_SKELETON, theme.metrics.pad),
                        ))
                        .child(
                            row().pt(rems(theme, ACTIONS_DROP)).child(
                                Skeleton::new()
                                    .build(theme)
                                    .size(ACTION_SKELETON, theme.metrics.control),
                            ),
                        ),
                )
                .a11y_hidden();
        }

        let mut cover = Cover::new(ui.artwork(), self.cover.as_deref())
            .size(art)
            .radius(corner);
        if self.circle {
            cover = cover.circle();
        }
        if let Some(fallback) = self.fallback {
            cover = cover.fallback(fallback);
        }
        if self.accent {
            cover = cover.accent();
        }
        let mut cover = row().key("hero-cover").child(cover.build(theme));
        let mut title = text(self.title.clone())
            .key("title")
            .role("heading")
            .level(1)
            .label(self.title.clone())
            .font(font(theme, Text::Display, Weight::BOLD))
            .color(theme.foreground.color())
            .truncate()
            .shrink()
            .min_w(0.);
        if !self.title.is_empty() {
            title = title.tip_label(self.title.clone(), Perch::Below);
        }
        if let Some(on_context) = self.on_context {
            let menu = on_context.clone();
            cover = cover.on_secondary_press(move |ui: &mut Ui, at: PointerPress, tx: &mut Tx| {
                menu(ui, at, tx)
            });
            title = title.on_secondary_press(move |ui: &mut Ui, at: PointerPress, tx: &mut Tx| {
                on_context(ui, at, tx)
            });
        }
        let badge = self.explicit.then(|| {
            row()
                .key("explicit")
                .role("image")
                .label(t!("song-explicit").into_owned())
                .child(ExplicitBadge::new().build(theme))
        });
        // The line keeps GPUI's height of the title, which Rune would round up to a whole point.
        let heading = row()
            .key("hero-title")
            .h(theme.text(Text::Display) * ui::LEADING)
            .center_y()
            .gap(rems(theme, TITLE_GAP))
            .min_w(0.)
            .child(title)
            .children(badge);
        hero.child(cover).child(
            lines.child(heading).children(self.meta).children(
                self.actions
                    .map(|actions| row().pt(rems(theme, ACTIONS_DROP)).child(actions)),
            ),
        )
    }
}
