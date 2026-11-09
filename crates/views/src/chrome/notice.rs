//! The column of cards in the top left corner of the workspace: one for every missing system
//! requirement, such as no sound output, then the update on offer. Every card can be closed
//! and told never to show again.

use std::rc::Rc;

use i18n::t;
use music::SoundServer;
use rune::{Cx, El, Transition, Tx, col, icon, row, text};
use state::{Requirement, UpdateState};
use ui::{Button, Text, Theme, Weight, corners};

use crate::chrome::player_bar::rems;
use crate::{Ui, act, act_tx};

const ICON: f32 = 16.;
/// The title's line height as a multiple of its size, which the icon centres on.
const LEADING: f32 = 1.4;
const REACH: f32 = 400.;
/// Where the README says what to install alongside Sonora.
const INSTALL: &str = "https://github.com/sonorahq/sonora#linux";
/// GPUI's `shadow_md`, the alert's lift off the page, as y, blur, spread and the black's alpha.
const SHADOW: [[f32; 4]; 2] = [[4., 6., -1., 0.1], [2., 4., -2., 0.1]];
/// How far a card slides in from as it appears.
const ENTER: f32 = -12.;

type Handler = Rc<dyn Fn(&mut Ui)>;

/// One card's content. The layout is shared, so every card closes and silences the same way.
struct Card {
    key: &'static str,
    icon: &'static str,
    title: String,
    detail: Option<String>,
    close: Handler,
    never: String,
    silence: Handler,
    /// A link in the bottom row, such as the release notes.
    link: Option<El>,
    /// What the card is about, first in the bottom row.
    actions: Vec<El>,
}

/// The cards, or nothing while none is up. The workspace lays it over its top left corner.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let _ = (cx, layer);
    let theme = &ui.theme;
    let mut cards: Vec<Card> = ui
        .store
        .requirements
        .shown(&ui.store.settings, &ui.store.session)
        .into_iter()
        .map(|requirement| requirement_card(requirement, theme))
        .collect();
    cards.extend(update_card(ui));
    let column = col()
        .key("app-notices")
        .role("group")
        .absolute(theme.metrics.pad, theme.metrics.pad)
        .w(REACH)
        .gap(rems(theme, 0.5));
    if cards.is_empty() {
        return column;
    }
    column.children(cards.into_iter().map(|card| card_element(card, theme)))
}

fn requirement_card(requirement: Requirement, theme: &Theme) -> Card {
    let (key, icon, title, detail) = match requirement {
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
    Card {
        key,
        icon,
        title: title.into_owned(),
        detail: Some(detail.into_owned()),
        close: Rc::new(move |ui: &mut Ui| ui.store.requirements.dismiss(requirement)),
        never: t!("notice-never").into_owned(),
        silence: Rc::new(move |ui: &mut Ui| ui.store.silence_requirement(requirement)),
        link: Some(link(
            &format!("{key}-help"),
            t!("requirement-help").into_owned(),
            INSTALL,
            theme,
        )),
        actions: Vec::new(),
    }
}

fn update_card(ui: &Ui) -> Option<Card> {
    let theme = &ui.theme;
    let updates = &ui.store.updates;
    let state = updates.state();
    let installable = updates.installable();
    let (version, page) = match state {
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
        true => t!("update-detail", running = state::version()),
        false => t!("update-detail-notes", running = state::version()),
    });
    let notes = (!page.is_empty()).then(|| {
        link(
            "update-notes",
            t!("update-notes").into_owned(),
            &page,
            theme,
        )
    });
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
                .on_press(act(|ui: &mut Ui| {
                    ui.store.updates.install(&mut ui.store.cx)
                }))
                .build(theme)
        })
        .into_iter()
        .collect();
    Some(Card {
        key: "update",
        icon: "icons/refresh-cw.svg",
        title: title.into_owned(),
        detail: detail.map(|detail| detail.into_owned()),
        close: Rc::new(|ui: &mut Ui| ui.store.updates.dismiss()),
        never: t!("update-never").into_owned(),
        silence: Rc::new(|ui: &mut Ui| {
            ui.store.settings.set_check_updates(false, &mut ui.store.cx);
            ui.store.updates.dismiss();
        }),
        link: notes,
        actions,
    })
}

/// Draws one card the way shadcn draws an alert: the icon in its own column, the title, detail
/// and buttons stacked beside it on one left edge, and the close button in the top right corner.
/// It slides down into place as it appears and fades as it goes.
fn card_element(card: Card, theme: &Theme) -> El {
    let Card {
        key,
        icon: path,
        title,
        detail,
        close,
        never,
        silence,
        link,
        actions,
    } = card;
    let outer = corners(theme.radius);
    // concentric corners: the close button sits one padding inside the card
    let inner = (outer - theme.metrics.pad).max(0.);
    let line = theme.text(Text::Body) * LEADING;

    let mut words = col()
        .grow(1.)
        .shrink()
        .min_w(0.)
        .gap(rems(theme, 0.25))
        .child(
            text(title.clone())
                .font(ui::font(theme, Text::Body, Weight::MEDIUM))
                .color(theme.foreground.color())
                .h(line)
                .center_y()
                .truncate()
                .min_w(0.)
                .pr(theme.metrics.control_small)
                .a11y_hidden(),
        );
    if let Some(detail) = detail.clone() {
        words = words.child(
            text(detail)
                .font(ui::font(theme, Text::Small, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .wrap()
                .a11y_hidden(),
        );
    }
    words = words.child(
        row()
            .flex_wrap()
            .center_y()
            .gap(rems(theme, 0.5))
            .mt(rems(theme, 0.5))
            .children(actions)
            .children(link)
            .child(
                Button::new(format!("{key}-never"))
                    .ghost()
                    .small()
                    .label(never)
                    .on_press(act(move |ui: &mut Ui| silence(ui)))
                    .build(theme),
            ),
    );
    let mut card = shadow_md(row())
        .key(format!("{key}-card"))
        .role("alert")
        .label(title)
        .items_start()
        .gap(rems(theme, 0.75))
        .px(rems(theme, 1.))
        .py(rems(theme, 0.75))
        .radius(outer)
        .border(1., theme.border.color())
        .fill(theme.popover.color())
        .transition(Transition::offset(0., ENTER).combined(Transition::opacity()))
        .child(
            // as tall as the title's line, so the icon centres on it
            row().h(line).center_y().child(
                icon(icons::get(path), ICON)
                    .color(theme.foreground.color())
                    .a11y_hidden(),
            ),
        )
        .child(words)
        .child(
            row()
                .pin(None, Some(theme.metrics.pad), Some(theme.metrics.pad), None)
                .child(
                    Button::new(format!("{key}-dismiss"))
                        .ghost()
                        .small()
                        .icon("icons/x.svg")
                        .tint(theme.muted_foreground)
                        .tooltip(t!("common-dismiss"))
                        .on_press(act(move |ui: &mut Ui| close(ui)))
                        .build(theme)
                        .radius(inner),
                ),
        );
    if let Some(detail) = detail {
        card = card.description(detail);
    }
    card
}

/// An outlined button that opens `url`, with the external-link arrow after its label.
fn link(key: &str, label: String, url: &str, theme: &Theme) -> El {
    let url = url.to_owned();
    Button::new(key)
        .outline()
        .small()
        .label(label)
        .trailing("icons/external-link.svg")
        .on_press_tx(act_tx(move |_: &mut Ui, tx: &mut Tx| {
            if let Err(error) = tx.open_url(&url) {
                log::warn!("notice: cannot open {url}: {error}");
            }
        }))
        .build(theme)
}

/// Lifts `e` off the page the way GPUI's `shadow_md` does.
fn shadow_md(e: El) -> El {
    SHADOW.into_iter().fold(e, |e, [y, blur, spread, alpha]| {
        e.box_shadow(rune::Shadow {
            x: 0.,
            y,
            blur,
            spread,
            color: rune::Color::BLACK.alpha(alpha),
            inset: false,
        })
    })
}
