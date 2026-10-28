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
use ui::{ActiveTheme as _, Button, SMALL_INSET, Text, corners};

const ICON: Pixels = px(18.);
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
    /// What the card is about, in the bottom row after the never-again button.
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

/// Draws one card: icon, a one-line title and a close button on top, then the detail, and a row
/// at the bottom spreading the never-again button, the link and the card's own actions across
/// its width. The detail is inset as far as a small button's label, so it lines up with the
/// buttons under it. The card takes every click and hover over it, so nothing under it reacts.
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
    // Concentric corners: the buttons sit one padding inside the card.
    let inner = (outer - theme.metrics.pad).max(Pixels::ZERO);

    div()
        .id(SharedString::from(format!("{id}-card")))
        .role(Role::Alert)
        .aria_label(title.clone())
        .occlude()
        .flex()
        .flex_col()
        .gap_2()
        .p(theme.metrics.pad)
        .rounded(outer)
        .border_1()
        .border_color(theme.border)
        .shadow_md()
        .bg(theme.popover)
        .text_color(theme.foreground)
        .child(
            div()
                .flex()
                .items_start()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w_0()
                        .child(
                            svg()
                                .path(icons::path(icon))
                                .size(ICON)
                                .flex_none()
                                .text_color(theme.primary),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(title),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("{id}-dismiss")))
                        .ghost()
                        .small()
                        .icon("icons/x.svg")
                        .rounded(inner)
                        .tooltip("common-dismiss")
                        .on_click(close),
                ),
        )
        .when_some(detail, |this, detail| {
            this.child(
                div()
                    .px(SMALL_INSET)
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
                .justify_between()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("{id}-never")))
                        .ghost()
                        .small()
                        .icon("icons/x.svg")
                        .label(never)
                        .rounded(inner)
                        .on_click(silence),
                )
                .children(link.map(|link| link.rounded(inner)))
                .children(actions.into_iter().map(|action| action.rounded(inner))),
        )
        .into_any_element()
}

/// A link-style button that opens `url`, with the external-link arrow before its label.
fn link(id: SharedString, label: SharedString, url: String) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .label(label)
        .icon("icons/external-link.svg")
        .on_click(move |_, _, cx| cx.open_url(&url))
}
