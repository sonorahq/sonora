use std::rc::Rc;

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, FontWeight, MouseButton, MouseDownEvent,
    Pixels, Rems, SharedString, Stateful, Window, div, px, relative, rems,
};
use i18n::t;
use music::{Album, Track};
use state::{Origin, Playback};
use ui::{
    ActiveTheme as _, Artwork, Button, ExplicitBadge, LEADING, MenuItem, Perch, Pin, Pinnable as _,
    Skeleton, TableState, Tails as _, Text, Tipped as _, spoken, upper,
};

use crate::shared::tracks::{self, TrackSource};

/// How wide the skeleton bars of a pending hero reach, for the title, the meta line and the
/// row of actions.
const TITLE_SKELETON: Pixels = px(240.);
const META_SKELETON: Pixels = px(180.);
const ACTION_SKELETON: Pixels = px(96.);
/// The space between the cover and the text beside it.
const COVER_GAP: Rems = rems(1.25);
/// The space between the buttons in a row of hero actions.
pub(crate) const ACTION_GAP: Rems = rems(0.5);

/// One copyright line, with a © in front unless the provider already led it with © or ℗.
pub(crate) fn copyright_notice(text: &str) -> SharedString {
    match text.starts_with(['©', '℗']) {
        true => SharedString::from(text.to_owned()),
        false => t!("song-copyright", notice = text),
    }
}

/// The notices an album page prints under its tracks: the provider's copyright lines, or the
/// label behind ℗ when the provider names only that. Empty when it names neither.
pub(crate) fn copyright_notices(album: &Album) -> Vec<SharedString> {
    let lines = album.copyright_lines();
    if !lines.is_empty() {
        return lines.into_iter().map(copyright_notice).collect();
    }
    match album.label_name() {
        None => Vec::new(),
        Some(label) => vec![t!("album-label", label = label)],
    }
}

pub(crate) fn release_date_label(value: &str) -> SharedString {
    let parts: Vec<_> = value.split('-').collect();
    if parts.len() != 3 {
        return SharedString::from(value.to_owned());
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
        _ => return SharedString::from(value.to_owned()),
    };
    let day = parts[2].trim_start_matches('0');

    t!("date-full", month = &month, day = day, year = parts[0])
}

/// The line of facts under a hero title, joined by bullets. Its text items reach screen readers
/// as labels, while an element item has to name itself.
#[derive(IntoElement, Default)]
pub(crate) struct HeroMetaStrip {
    items: Vec<MetaItem>,
}

/// One fact on a `HeroMetaStrip`.
enum MetaItem {
    Text(SharedString),
    Element(AnyElement),
}

impl HeroMetaStrip {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn text(mut self, text: impl Into<SharedString>) -> Self {
        self.items.push(MetaItem::Text(text.into()));
        self
    }

    pub(crate) fn item(mut self, item: impl IntoElement) -> Self {
        self.items.push(MetaItem::Element(item.into_any_element()));
        self
    }
}

impl RenderOnce for HeroMetaStrip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();
        let mut strip = div()
            .flex()
            .flex_wrap()
            .items_center()
            .min_w_0()
            .gap_1()
            .text_size(theme.text(Text::Small))
            .text_color(theme.muted_foreground);

        for (index, item) in self.items.into_iter().enumerate() {
            let item = match item {
                MetaItem::Text(text) => spoken(("hero-meta", index), text)
                    .flex_none()
                    .into_any_element(),
                MetaItem::Element(item) => item,
            };
            strip = strip.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .when(index > 0, |this| this.child("•"))
                    .child(item),
            );
        }

        strip
    }
}

/// The tracks ready to play, or a collection to load when its button is pressed.
enum Listing {
    Owned(Vec<Track>),
    Listed(Entity<TableState<TrackSource>>),
    Origin(Option<Origin>),
}

impl Listing {
    fn can_start(&self, cx: &App) -> bool {
        match self {
            Self::Owned(tracks) => tracks.iter().any(|track| track.playable),
            Self::Listed(table) => tracks::first_playable(table, cx).is_some(),
            Self::Origin(origin) => origin.is_some(),
        }
    }

    fn holds(&self, id: &str, cx: &App) -> bool {
        match self {
            Self::Owned(tracks) => tracks.iter().any(|track| track.id.as_deref() == Some(id)),
            Self::Listed(table) => tracks::holds(table, id, cx),
            Self::Origin(_) => false,
        }
    }

    fn whence(&self, cx: &App) -> Option<Origin> {
        match self {
            Self::Owned(_) => None,
            Self::Listed(table) => tracks::whence(table, cx),
            Self::Origin(origin) => origin.clone(),
        }
    }

    /// Loads a collection on demand or starts the tracks already held by the page.
    fn start(&self, playback: &mut Playback, shuffle: bool, cx: &mut Context<Playback>) {
        let tracks = match self {
            Self::Origin(origin) => {
                if let Some(origin) = origin.clone() {
                    match shuffle {
                        true => playback.shuffle_origin(origin, cx),
                        false => playback.play_origin(origin, cx),
                    }
                }
                return;
            }
            Self::Owned(tracks) => tracks.clone(),
            Self::Listed(table) => tracks::ordered(table, cx),
        };
        let from = self.whence(cx);
        match shuffle {
            true => playback.shuffle_any(tracks, from, cx),
            false => playback.start_any(tracks, from, cx),
        }
    }
}

/// A page's Play or Shuffle button, using loaded tracks or an origin fetched on demand.
#[derive(IntoElement)]
pub(crate) struct HeroPlayButton {
    id: ElementId,
    listing: Listing,
    shuffled: bool,
    playback: Entity<Playback>,
}

impl HeroPlayButton {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        tracks: Vec<Track>,
        playback: Entity<Playback>,
    ) -> Self {
        Self {
            id: id.into(),
            listing: Listing::Owned(tracks),
            shuffled: false,
            playback,
        }
    }

    /// Fetches the whole collection on press instead of playing only the page's loaded rows.
    pub(crate) fn load_origin(mut self, origin: Option<Origin>) -> Self {
        self.listing = Listing::Origin(origin);
        self
    }

    /// The Shuffle button beside Play: an icon-only outline button that turns shuffle on and
    /// starts the tracks from a random one, never mirroring the transport state.
    pub(crate) fn shuffle(
        id: impl Into<ElementId>,
        tracks: Vec<Track>,
        playback: Entity<Playback>,
    ) -> Self {
        Self {
            shuffled: true,
            ..Self::new(id, tracks, playback)
        }
    }

    /// `shuffle` over a table's rows, in their displayed order.
    pub(crate) fn shuffle_listed(
        id: impl Into<ElementId>,
        table: &Entity<TableState<TrackSource>>,
        playback: Entity<Playback>,
    ) -> Self {
        Self {
            shuffled: true,
            ..Self::listed(id, table, playback)
        }
    }

    pub(crate) fn listed(
        id: impl Into<ElementId>,
        table: &Entity<TableState<TrackSource>>,
        playback: Entity<Playback>,
    ) -> Self {
        Self {
            id: id.into(),
            listing: Listing::Listed(table.clone()),
            shuffled: false,
            playback,
        }
    }

    /// The labels the Play button switches between. They are kept short and its width fits the
    /// wider of them, so the buttons beside it stay put as playback starts and stops.
    fn labels() -> [SharedString; 2] {
        [t!("common-play"), t!("play-pause")]
    }

    /// How wide the button draws, the same whatever its label says.
    pub(crate) fn width(&self, window: &Window) -> Pixels {
        match self.shuffled {
            true => Button::width(std::iter::empty::<SharedString>(), window),
            false => Button::width(Self::labels(), window),
        }
    }

    /// The shuffle as a menu item, for a hero whose Shuffle button folded into its overflow
    /// menu.
    pub(crate) fn shuffle_item(self, cx: &App) -> MenuItem {
        let disabled = !self.listing.can_start(cx);
        let listing = self.listing;
        let playback = self.playback;

        MenuItem::new("hero-shuffle", t!("play-shuffle"))
            .icon("icons/shuffle.svg")
            .when(disabled, MenuItem::disabled)
            .on_click(move |_, _, cx| {
                playback.update(cx, |playback, cx| listing.start(playback, true, cx));
            })
    }

    fn shuffle_button(self, cx: &App) -> Button {
        let disabled = !self.listing.can_start(cx);
        let listing = self.listing;
        let playback = self.playback;

        Button::new(self.id)
            .icon("icons/shuffle.svg")
            .tooltip("play-shuffle")
            .outline()
            .disabled(disabled)
            .on_click(move |_, _, cx| {
                playback.update(cx, |playback, cx| listing.start(playback, true, cx));
            })
    }
}

impl RenderOnce for HeroPlayButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.shuffled {
            return div().flex().child(self.shuffle_button(cx));
        }
        let playing = {
            let playback = self.playback.read(cx);
            match self.listing.whence(cx) {
                Some(origin) => playback.playing_from(&origin, cx),
                None => playback
                    .track()
                    .and_then(|track| track.id.as_deref())
                    .filter(|current| self.listing.holds(current, cx))
                    .and_then(|_| playback.control()),
            }
        };
        let [play, pause] = Self::labels();
        let reserve = [play.clone(), pause.clone()];
        let (label, icon) = match playing {
            Some(true) => (pause, "icons/pause.svg"),
            Some(false) | None => (play, "icons/play.svg"),
        };
        let disabled = !self.listing.can_start(cx);
        let listing = self.listing;
        let playback = self.playback;

        div().flex().child(
            Button::new(self.id)
                .label(label)
                .reserve(reserve)
                .icon(icon)
                .primary()
                .disabled(disabled)
                .on_click(move |_, _, cx| {
                    playback.update(cx, |playback, cx| match playing {
                        Some(_) => playback.toggle_play(cx),
                        None => listing.start(playback, false, cx),
                    });
                }),
        )
    }
}

type DragStart = Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub(crate) struct PageHero {
    id: ElementId,
    title: SharedString,
    cover: Option<String>,
    fallback: Option<SharedString>,
    accent: bool,
    eyebrow: Option<SharedString>,
    meta: Option<AnyElement>,
    actions: Option<AnyElement>,
    circle: bool,
    explicit: bool,
    drag_start: Option<DragStart>,
    pin: Option<Pin>,
    pending: bool,
}

impl PageHero {
    pub(crate) fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            cover: None,
            fallback: None,
            accent: false,
            eyebrow: None,
            meta: None,
            actions: None,
            circle: false,
            explicit: false,
            drag_start: None,
            pin: None,
            pending: false,
        }
    }

    /// The width a hero `width` wide leaves beside its cover for the title and the actions.
    pub(crate) fn room(width: Pixels, window: &Window, cx: &App) -> Pixels {
        width - cx.theme().metrics.cover - COVER_GAP.to_pixels(window.rem_size())
    }

    pub(crate) fn pin(mut self, pin: Option<Pin>) -> Self {
        self.pin = pin;
        self
    }

    pub(crate) fn drag_start(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.drag_start = Some(Box::new(handler));
        self
    }

    pub(crate) fn cover(mut self, cover: Option<String>) -> Self {
        self.cover = cover;
        self
    }

    pub(crate) fn fallback(mut self, icon: impl Into<SharedString>) -> Self {
        self.fallback = Some(icon.into());
        self
    }

    pub(crate) fn accent(mut self) -> Self {
        self.accent = true;
        self
    }

    pub(crate) fn eyebrow(mut self, eyebrow: impl Into<SharedString>) -> Self {
        self.eyebrow = Some(eyebrow.into());
        self
    }

    pub(crate) fn meta(mut self, meta: impl IntoElement) -> Self {
        self.meta = Some(meta.into_any_element());
        self
    }

    pub(crate) fn actions(mut self, actions: impl IntoElement) -> Self {
        self.actions = Some(actions.into_any_element());
        self
    }

    pub(crate) fn circle(mut self) -> Self {
        self.circle = true;
        self
    }

    pub(crate) fn explicit(mut self, explicit: bool) -> Self {
        self.explicit = explicit;
        self
    }

    /// Draws the cover, title, meta line and actions as skeletons while the page has nothing
    /// to fill them with yet. The eyebrow stays, since it names the kind of page.
    pub(crate) fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }
}

impl RenderOnce for PageHero {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = *cx.theme();

        let art = theme.metrics.cover;
        let drag_start = self.drag_start.map(Rc::from);
        let starter = drag_start.clone();

        if self.pending {
            return div()
                .id(self.id)
                .flex()
                .flex_none()
                .items_end()
                .gap(COVER_GAP)
                .pb_6()
                .child(
                    Skeleton::new()
                        .flex_none()
                        .size(art)
                        .rounded(theme.radius * 1.5)
                        .when(self.circle, |skeleton| skeleton.rounded_full()),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .h(art)
                        .justify_end()
                        .gap_2()
                        .line_height(relative(LEADING))
                        .children(self.eyebrow.map(|eyebrow| eyebrow_label(eyebrow, cx)))
                        .child(
                            skeleton_line(theme.text(Text::Display)).child(
                                Skeleton::new()
                                    .w(TITLE_SKELETON)
                                    .h(theme.text(Text::Display)),
                            ),
                        )
                        .child(
                            skeleton_line(theme.text(Text::Small))
                                .child(Skeleton::new().w(META_SKELETON).h(theme.metrics.pad)),
                        )
                        .child(
                            div()
                                .pt_1()
                                .child(Skeleton::new().w(ACTION_SKELETON).h(theme.metrics.control)),
                        ),
                );
        }

        div()
            .id(self.id.clone())
            .when_some(self.pin, |hero, pin| hero.pin(pin))
            .flex()
            .flex_none()
            .items_end()
            .gap(COVER_GAP)
            .pb_6()
            .child(
                div()
                    .flex_none()
                    .when_some(starter, |this, drag_start: Rc<DragStart>| {
                        this.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                            drag_start(event, window, cx)
                        })
                    })
                    .child(
                        Artwork::new(self.cover)
                            .size(art)
                            .corner_radius(theme.radius * 1.5)
                            .when(self.circle, Artwork::circle)
                            .when_some(self.fallback, Artwork::fallback)
                            .when(self.accent, Artwork::accent),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h(art)
                    .justify_end()
                    .gap_2()
                    .line_height(relative(LEADING))
                    .children(self.eyebrow.map(|eyebrow| eyebrow_label(eyebrow, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .min_w_0()
                            .text_size(theme.text(Text::Display))
                            .font_weight(FontWeight::BOLD)
                            .child(
                                div()
                                    .id((self.id.clone(), "title"))
                                    .role(Role::Heading)
                                    .aria_level(1)
                                    .aria_label(self.title.clone())
                                    .min_w_0()
                                    .truncate()
                                    .tails()
                                    .when(!self.title.is_empty(), |this| {
                                        this.tip_label(self.title.clone(), Perch::Below)
                                    })
                                    .when_some(drag_start, |this, drag_start: Rc<DragStart>| {
                                        this.on_mouse_down(
                                            MouseButton::Right,
                                            move |event, window, cx| drag_start(event, window, cx),
                                        )
                                    })
                                    .child(self.title),
                            )
                            .when(self.explicit, |this| {
                                this.child(
                                    div()
                                        .id((self.id.clone(), "explicit"))
                                        .role(Role::Image)
                                        .aria_label(t!("song-explicit"))
                                        .flex_none()
                                        .child(ExplicitBadge::new()),
                                )
                            }),
                    )
                    .children(self.meta)
                    .children(self.actions.map(|actions| div().pt_1().child(actions))),
            )
    }
}

/// The kind of page above the hero title, drawn in capitals. Screen readers get the label as
/// written, since a reader may spell out text in capitals letter by letter.
fn eyebrow_label(label: SharedString, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();

    div()
        .id("hero-eyebrow")
        .role(Role::Label)
        .aria_value(label.clone())
        .text_size(theme.text(Text::Small))
        .text_color(theme.muted_foreground)
        .font_weight(FontWeight::SEMIBOLD)
        .child(upper(label))
}

/// A row as tall as one line of text at `size`, so the skeleton bar inside it takes the room of
/// the text it stands in for and the hero keeps its layout once the page loads.
fn skeleton_line(size: Pixels) -> Div {
    div().flex().items_center().h(size * LEADING)
}
