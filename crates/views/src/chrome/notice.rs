//! The column of cards in the top left corner: one for every missing system requirement, then
//! the update on offer. Every card is drawn like a toast, only larger. It drops in on a spring,
//! grows and shrinks to fit what it says, and fades away once closed. Every card can be closed
//! and told never to show again.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FontWeight, Hsla, Pixels, Render, SharedString,
    SpringConfig, Window, div, point, px, relative, svg,
};
use i18n::t;
use music::SoundServer;
use state::{AppSettings, Requirement, Requirements, Sonora, UpdateState, Updates};
use ui::{
    ActiveTheme as _, ArrowRing, Button, Linger, Springs, Sprung, Text, Unfold, corners, spring,
};

const ICON: Pixels = px(18.);
/// The title's line height as a multiple of its size, which the icon centres on.
const LEADING: f32 = 1.4;
const REACH: Pixels = px(420.);
/// How thick the download's progress bar is.
const BAR: Pixels = px(4.);
/// How far above its place a card starts as it drops in, in pixels.
const DROP: f32 = 20.;
/// How much smaller a card starts as it drops in.
const ZOOM: f32 = 0.06;
/// The drop, with a little bounce so the card settles into place.
const ARRIVE: SpringConfig = spring(0.55, 0.2);
/// How close to rest the drop has to come to count as landed.
const ARRIVE_REST: f32 = 0.001;
/// How small a new icon starts when the card turns to another state.
const SWAP_FROM: f32 = 0.4;
/// How close to full size a swapped icon has to come to count as landed.
const SWAP_REST: f32 = 0.002;
/// How close the progress bar has to come to the amount done to stop moving.
const FILL_REST: f32 = 0.001;
/// Where the README says what to install alongside Sonora.
const INSTALL: &str = "https://github.com/sonorahq/sonora#linux";

type Handler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

pub(crate) struct AppNotice {
    updates: Entity<Updates>,
    requirements: Entity<Requirements>,
    settings: Entity<AppSettings>,
}

/// One card's content. The layout is shared, so every card closes and silences the same way.
struct Card {
    id: &'static str,
    mark: Mark,
    title: SharedString,
    detail: Option<SharedString>,
    close: Handler,
    /// The quiet button that stops the card for good, with its label. A card for work under way
    /// has none.
    never: Option<(SharedString, Handler)>,
    /// A link in the bottom row, such as the release notes.
    link: Option<Button>,
    /// What the card is about, first in the bottom row.
    actions: Vec<Button>,
    /// How far a download has got, from 0 to 1, drawn as a bar under the detail. `Some(None)`
    /// draws an empty bar while the size is not known yet.
    progress: Option<Option<f32>>,
}

/// What a card leads with: an icon in the colour of what it reports, or a ring filling as a
/// download goes.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Icon(&'static str, Tone),
    Ring(Option<f32>),
}

/// What a card reports, which picks its icon's colour.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Success,
    Warning,
    Error,
}

/// What a card keeps between frames: where it is in its drop, how far its icon has grown back
/// after a change of state, how far its progress bar has filled and how tall it stands.
struct Motion {
    arrive: Sprung,
    /// The icon the card showed last frame, so a change pops the new one in.
    mark: Option<&'static str>,
    swap: Sprung,
    fill: Sprung,
    height: Unfold,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            arrive: Sprung::at(1.),
            mark: None,
            swap: Sprung::at(1.),
            fill: Sprung::at(0.),
            height: Unfold::default(),
        }
    }
}

impl Mark {
    /// Tells one mark from another, so a card that turns to another state pops its icon in.
    fn key(self) -> &'static str {
        match self {
            Self::Icon(icon, _) => icon,
            Self::Ring(_) => "ring",
        }
    }
}

impl Tone {
    fn color(self, cx: &App) -> Hsla {
        let theme = cx.theme();
        match self {
            Self::Info => theme.status_info,
            Self::Success => theme.status_success,
            Self::Warning => theme.status_warning,
            Self::Error => theme.status_error,
        }
    }
}

impl AppNotice {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let sonora = Sonora::global(cx);
        let updates = sonora.updates.clone();
        let requirements = sonora.requirements.clone();
        let settings = sonora.settings.clone();
        cx.observe(&updates, |_, _, cx| cx.notify()).detach();
        cx.observe(&requirements, |_, _, cx| cx.notify()).detach();
        Self {
            updates,
            requirements,
            settings,
        }
    }

    fn requirement(&self, requirement: Requirement) -> Card {
        let (id, icon, title, detail) = match requirement {
            Requirement::AudioBridge(server) => (
                "requirement-audio-bridge",
                "icons/volume-x.svg",
                t!("requirement-audio-bridge-title"),
                t!(
                    "requirement-audio-bridge",
                    server = match server {
                        SoundServer::PipeWire => "pipewire",
                        SoundServer::PulseAudio => "pulseaudio",
                    }
                ),
            ),
            Requirement::AudioOutput => (
                "requirement-audio-output",
                "icons/volume-x.svg",
                t!("requirement-audio-output-title"),
                t!("requirement-audio-output"),
            ),
            Requirement::WebEngine => (
                "requirement-web-engine",
                "icons/circle-alert.svg",
                t!("requirement-web-engine-title"),
                t!("requirement-web-engine"),
            ),
        };
        let close = self.requirements.clone();
        let silence = self.requirements.clone();

        Card {
            id,
            mark: Mark::Icon(icon, Tone::Warning),
            title,
            detail: Some(detail),
            close: Box::new(move |_, _, cx| {
                close.update(cx, |requirements, cx| requirements.dismiss(requirement, cx));
            }),
            never: Some((
                t!("notice-never"),
                Box::new(move |_, _, cx| {
                    silence.update(cx, |requirements, cx| requirements.silence(requirement, cx));
                }),
            )),
            link: Some(link(
                SharedString::from(format!("{id}-help")),
                t!("requirement-help"),
                INSTALL.to_owned(),
            )),
            actions: Vec::new(),
            progress: None,
        }
    }

    fn update(&self, cx: &App) -> Option<Card> {
        let updates = self.updates.read(cx);
        let state = updates.state().clone();
        let installable = updates.installable();
        let progress = updates.progress();
        let running = state::version();

        let install = self.updates.clone();
        let (mark, title, detail, page, action) = match &state {
            UpdateState::Quiet => return None,
            UpdateState::Offered(release) => (
                Mark::Icon("icons/circle-arrow-down.svg", Tone::Info),
                t!("update-available", version = release.version.as_str()),
                Some(match installable {
                    true => t!("update-detail", running = running),
                    false => t!("update-detail-notes", running = running),
                }),
                Some(release.page.clone()),
                installable.then(|| {
                    Button::new("update-now")
                        .primary()
                        .small()
                        .label(t!("update-now"))
                        .on_click(move |_, _, cx| {
                            install.update(cx, |updates, cx| updates.install(cx));
                        })
                }),
            ),
            UpdateState::Fetching(release) => (
                Mark::Ring(progress),
                t!("update-downloading", name = release.version.as_str()),
                Some(t!("update-fetching-detail")),
                Some(release.page.clone()),
                None,
            ),
            UpdateState::Ready(release) => (
                Mark::Icon("icons/circle-check.svg", Tone::Success),
                t!("update-ready", version = release.version.as_str()),
                Some(t!("update-ready-detail", running = running)),
                Some(release.page.clone()),
                Some(
                    Button::new("update-restart")
                        .primary()
                        .small()
                        .label(t!("update-restart"))
                        .on_click(move |_, _, cx| {
                            install.update(cx, |updates, cx| updates.restart(cx));
                        }),
                ),
            ),
            UpdateState::Failed => (
                Mark::Icon("icons/octagon-x.svg", Tone::Error),
                t!("update-failed"),
                None,
                None,
                None,
            ),
        };
        let fetching = matches!(state, UpdateState::Fetching(_));

        let close = self.updates.clone();
        let silence = self.updates.clone();
        let settings = self.settings.clone();
        Some(Card {
            id: "update",
            mark,
            title,
            detail,
            close: Box::new(move |_, _, cx| {
                close.update(cx, |updates, cx| updates.dismiss(cx));
            }),
            never: (!fetching).then(|| {
                let handler: Handler = Box::new(move |_, _, cx| {
                    settings.update(cx, |settings, cx| settings.set_check_updates(false, cx));
                    silence.update(cx, |updates, cx| updates.dismiss(cx));
                });
                (t!("update-never"), handler)
            }),
            link: page
                .map(|page| link(SharedString::from("update-notes"), t!("update-notes"), page)),
            actions: action.into_iter().collect(),
            progress: fetching.then_some(progress),
        })
    }
}

impl Render for AppNotice {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let mut cards: Vec<Card> = self
            .requirements
            .read(cx)
            .shown(cx)
            .into_iter()
            .map(|requirement| self.requirement(requirement))
            .collect();
        cards.extend(self.update(cx));
        if cards.is_empty() {
            return div().into_any_element();
        }
        // The download writes its progress off the main thread, so the bar follows it by
        // drawing again on every frame while it runs.
        if matches!(self.updates.read(cx).state(), UpdateState::Fetching(_)) {
            window.request_animation_frame();
        }

        div()
            .id("app-notices")
            .role(Role::Group)
            .absolute()
            .top(theme.metrics.pad)
            .left(theme.metrics.pad)
            .w(REACH)
            .flex()
            .flex_col()
            .gap_2()
            .children(
                cards
                    .into_iter()
                    .map(|card| card_element(card, window, cx))
                    .collect::<Vec<_>>(),
            )
            .into_any_element()
    }
}

/// Draws one card the way a toast is drawn, only larger: the icon in its own column in the
/// colour of what the card reports, the title, detail and buttons stacked beside it on one left
/// edge, and the close button in the top right corner. It drops in from above on a spring,
/// springs to its new height when what it says changes, and fades out once it is gone. The card
/// takes every click and hover over it, so nothing under it reacts.
fn card_element(card: Card, window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = *cx.theme();
    let Card {
        id,
        mark,
        title,
        detail,
        close,
        never,
        link,
        actions,
        progress,
    } = card;

    let motion = window.use_keyed_state(SharedString::from(format!("{id}-motion")), cx, |_, _| {
        Motion::default()
    });
    let target = progress.flatten().unwrap_or(0.);
    let (hidden, swap, fill) = motion.update(cx, |motion, cx| {
        let key = mark.key();
        if motion.mark.is_some_and(|last| last != key) {
            motion.swap.jump(SWAP_FROM);
        }
        motion.mark = Some(key);
        (
            motion.arrive.follow(0., ARRIVE, ARRIVE_REST, window, cx),
            motion
                .swap
                .follow(1., Springs::RELEASE, SWAP_REST, window, cx),
            motion
                .fill
                .follow(target, Springs::RESIZE, FILL_REST, window, cx),
        )
    });

    let outer = corners(theme.radius);
    // Concentric corners: the close button sits one padding inside the card.
    let inner = (outer - theme.metrics.pad).max(Pixels::ZERO);
    let line = theme.text(Text::Body) * LEADING;
    let pad = theme.metrics.pad * 2;

    let leading = match mark {
        Mark::Icon(icon, tone) => svg()
            .path(icons::path(icon))
            .size(ICON)
            .flex_none()
            .text_color(tone.color(cx))
            .into_any_element(),
        Mark::Ring(done) => ArrowRing::new(SharedString::from(format!("{id}-ring")), done)
            .color(theme.status_info)
            .into_any_element(),
    };
    let percent = progress.map(|done| {
        t!(
            "unit-percent",
            value = (done.unwrap_or(0.) * 100.).floor() as usize
        )
    });
    // The percentage drawn beside the progress bar is read after the detail.
    let about = match (detail.clone(), percent.clone()) {
        (Some(detail), Some(percent)) => Some(SharedString::from(format!("{detail}, {percent}"))),
        (detail, percent) => detail.or(percent),
    };

    let body = div()
        .flex()
        .items_start()
        .gap_3()
        .p(pad)
        .child(
            // As tall as the title's line, so the icon centres on it.
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(line)
                .child(leading)
                .when(swap != 1., |this| {
                    this.layer_scale(swap.max(0.01))
                        .layer_scale_origin(point(0.5, 0.5))
                        .opacity(swap.clamp(0., 1.))
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .min_w_0()
                        .pr(theme.metrics.control_small)
                        .line_height(line)
                        .text_size(theme.text(Text::Body))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.popover_foreground)
                        .truncate()
                        .child(title.clone()),
                )
                .when_some(detail, |this, detail| {
                    this.child(
                        div()
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    )
                })
                .when_some(percent, |this, percent| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .mt_2()
                            .child(
                                div()
                                    .flex_1()
                                    .h(BAR)
                                    .rounded_full()
                                    .overflow_hidden()
                                    .bg(theme.muted)
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(fill.clamp(0., 1.)))
                                            .rounded_full()
                                            .bg(theme.status_info),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(theme.text(Text::Small))
                                    .text_color(theme.muted_foreground)
                                    .child(percent),
                            ),
                    )
                })
                .when(
                    !actions.is_empty() || link.is_some() || never.is_some(),
                    |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .mt_3()
                                .children(actions)
                                .children(link)
                                .children(never.map(|(label, silence)| {
                                    Button::new(SharedString::from(format!("{id}-never")))
                                        .ghost()
                                        .small()
                                        .label(label)
                                        .tint(theme.muted_foreground)
                                        .on_click(silence)
                                })),
                        )
                    },
                ),
        );
    let content = motion.read(cx).height.wrap(body, false, window, cx);

    let card = div()
        .id(SharedString::from(format!("{id}-card")))
        .role(Role::Alert)
        .aria_label(title)
        .when_some(about, |this, about| this.aria_description(about))
        .occlude()
        .relative()
        .w_full()
        .rounded(outer)
        .border_1()
        .border_color(theme.border)
        .shadow_lg()
        .bg(theme.popover)
        .text_size(theme.text(Text::Label))
        .text_color(theme.popover_foreground)
        .child(content)
        .child(
            div()
                .absolute()
                .top(pad + (line - theme.metrics.control_small) * 0.5)
                .right(theme.metrics.pad)
                .child(
                    Button::new(SharedString::from(format!("{id}-dismiss")))
                        .ghost()
                        .small()
                        .icon("icons/x.svg")
                        .tint(theme.muted_foreground)
                        .rounded(inner)
                        .tooltip("common-dismiss")
                        .on_click(close),
                ),
        )
        .when(hidden != 0., |this| {
            this.layer_translate(point(px(0.), px(-DROP * hidden)))
                .layer_scale(1. - ZOOM * hidden)
                .layer_scale_origin(point(0., 0.))
                .opacity((1. - hidden).clamp(0., 1.))
        });

    Linger::new(SharedString::from(format!("{id}-linger")), card).into_any_element()
}

/// An outlined button that opens `url`, with the external-link arrow after its label.
fn link(id: SharedString, label: SharedString, url: String) -> Button {
    Button::new(id)
        .outline()
        .small()
        .label(label)
        .trailing("icons/external-link.svg")
        .on_click(move |_, _, cx| cx.open_url(&url))
}
