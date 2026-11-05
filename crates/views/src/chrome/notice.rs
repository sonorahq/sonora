//! The column of cards in the top left corner: one for every missing system requirement, then
//! the update on offer. Every card can be closed and told never to show again.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FontWeight, Pixels, Render, SharedString, Window,
    div, px, svg,
};
use i18n::t;
use music::SoundServer;
use state::{AppSettings, Requirement, Requirements, Sonora, UpdateState, Updates};
use ui::{ActiveTheme as _, Button, Text, corners};

const ICON: Pixels = px(16.);
/// The title's line height as a multiple of its size, which the icon centres on.
const LEADING: f32 = 1.4;
const REACH: Pixels = px(400.);
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
    icon: &'static str,
    title: SharedString,
    detail: Option<SharedString>,
    close: Handler,
    never: SharedString,
    silence: Handler,
    /// A link in the bottom row, such as the release notes.
    link: Option<Button>,
    /// What the card is about, first in the bottom row.
    actions: Vec<Button>,
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
            icon,
            title,
            detail: Some(detail),
            close: Box::new(move |_, _, cx| {
                close.update(cx, |requirements, cx| requirements.dismiss(requirement, cx));
            }),
            never: t!("notice-never"),
            silence: Box::new(move |_, _, cx| {
                silence.update(cx, |requirements, cx| requirements.silence(requirement, cx));
            }),
            link: Some(link(
                SharedString::from(format!("{id}-help")),
                t!("requirement-help"),
                INSTALL.to_owned(),
            )),
            actions: Vec::new(),
        }
    }

    fn update(&self, cx: &App) -> Option<Card> {
        let updates = self.updates.read(cx);
        let state = updates.state().clone();
        let installable = updates.installable();
        let (version, page) = match &state {
            UpdateState::Quiet => return None,
            UpdateState::Offered(release) => (release.version.clone(), release.page.clone()),
            _ => (String::new(), String::new()),
        };
        let working = matches!(state, UpdateState::Fetching);
        let failed = matches!(state, UpdateState::Failed);

        let title = match failed {
            true => t!("update-failed"),
            false => t!("update-available", version = version.as_str()),
        };
        let detail = (!failed).then(|| match installable {
            true => t!("update-detail", running = state::VERSION),
            false => t!("update-detail-notes", running = state::VERSION),
        });

        let close = self.updates.clone();
        let install = self.updates.clone();
        let silence = self.updates.clone();
        let settings = self.settings.clone();
        let notes = (!page.is_empty())
            .then(|| link(SharedString::from("update-notes"), t!("update-notes"), page));
        let actions = (installable && !failed)
            .then(|| {
                Button::new("update-now")
                    .primary()
                    .small()
                    .disabled(working)
                    .label(match working {
                        true => t!("update-working"),
                        false => t!("update-now"),
                    })
                    .on_click(move |_, _, cx| {
                        install.update(cx, |updates, cx| updates.install(cx));
                    })
            })
            .into_iter()
            .collect();

        Some(Card {
            id: "update",
            icon: "icons/refresh-cw.svg",
            title,
            detail,
            close: Box::new(move |_, _, cx| {
                close.update(cx, |updates, cx| updates.dismiss(cx));
            }),
            never: t!("update-never"),
            silence: Box::new(move |_, _, cx| {
                settings.update(cx, |settings, cx| settings.set_check_updates(false, cx));
                silence.update(cx, |updates, cx| updates.dismiss(cx));
            }),
            link: notes,
            actions,
        })
    }
}

impl Render for AppNotice {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .children(cards.into_iter().map(|card| card_element(card, cx)))
            .into_any_element()
    }
}

/// Draws one card the way shadcn draws an alert: the icon in its own column, the title, detail
/// and buttons stacked beside it on one left edge, and the close button in the top right corner.
/// The card takes every click and hover over it, so nothing under it reacts.
fn card_element(card: Card, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let Card {
        id,
        icon,
        title,
        detail,
        close,
        never,
        silence,
        link,
        actions,
    } = card;

    let outer = corners(theme.radius);
    // Concentric corners: the close button sits one padding inside the card.
    let inner = (outer - theme.metrics.pad).max(Pixels::ZERO);
    let line = theme.text(Text::Body) * LEADING;

    div()
        .id(SharedString::from(format!("{id}-card")))
        .role(Role::Alert)
        .aria_label(title.clone())
        .when_some(detail.clone(), |this, detail| this.aria_description(detail))
        .occlude()
        .relative()
        .flex()
        .items_start()
        .gap_3()
        .px_4()
        .py_3()
        .rounded(outer)
        .border_1()
        .border_color(theme.border)
        .shadow_md()
        .bg(theme.popover)
        .text_color(theme.foreground)
        .child(
            // As tall as the title's line, so the icon centres on it.
            div().flex().flex_none().items_center().h(line).child(
                svg()
                    .path(icons::path(icon))
                    .size(ICON)
                    .text_color(theme.foreground),
            ),
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
                        .font_weight(FontWeight::MEDIUM)
                        .truncate()
                        .child(title),
                )
                .when_some(detail, |this, detail| {
                    this.child(
                        div()
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.muted_foreground)
                            .child(detail),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .mt_2()
                        .children(actions)
                        .children(link)
                        .child(
                            Button::new(SharedString::from(format!("{id}-never")))
                                .ghost()
                                .small()
                                .label(never)
                                .on_click(silence),
                        ),
                ),
        )
        .child(
            div()
                .absolute()
                .top(theme.metrics.pad)
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
        .into_any_element()
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
