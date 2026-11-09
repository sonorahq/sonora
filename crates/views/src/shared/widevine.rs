//! The two questions before Google's Widevine module is installed: whether to download it,
//! then whether its terms are accepted. Both are `state::Drm`'s to ask; this only draws them.

use i18n::t;
use rune::{El, text};
use state::CdmState;
use ui::{Button, Modal, Text, Weight};

use crate::Ui;

const BORDER: f32 = 1.;

/// Whether the questions are up: from the moment one is asked until it is answered or the
/// install is done.
pub(crate) fn shows(ui: &Ui) -> bool {
    matches!(
        ui.store.drm.state(),
        CdmState::Wanted | CdmState::Offering | CdmState::Offered(_) | CdmState::Installing
    )
}

/// Not now, or Decline: puts the question away for this run.
fn decline(ui: &mut Ui) {
    ui.store.drm.dismiss();
    ui.settle();
}

/// Download, or Accept and install, whichever the question on screen asks.
fn proceed(ui: &mut Ui) {
    let store = &mut ui.store;
    let wanted = matches!(store.drm.state(), CdmState::Wanted);
    let offered = matches!(store.drm.state(), CdmState::Offered(_));
    match (wanted, offered) {
        (true, _) => store.drm.download(&mut store.cx),
        (_, true) => store.drm.accept(&mut store.cx),
        _ => {}
    }
    ui.settle();
}

/// Mounts the question, if one is asked, into `layer`.
pub(crate) fn view(ui: &Ui, layer: &mut Vec<El>) {
    if !shows(ui) {
        return;
    }
    let theme = &ui.theme;
    let drm = &ui.store.drm;
    let state = drm.state();
    let small = ui::font(theme, Text::Small, Weight::NORMAL);
    let wanted = match drm.replacing() {
        true => t!("widevine-prompt-replace"),
        false => t!("widevine-prompt-wanted"),
    };
    let (detail, note, license, action) = match state {
        CdmState::Wanted => (wanted, None, None, true),
        CdmState::Offering => (wanted, Some(t!("widevine-prompt-downloading")), None, false),
        CdmState::Offered(terms) => (
            t!("widevine-prompt-terms", version = terms.version.as_str()),
            None,
            Some(terms.license.clone()),
            true,
        ),
        _ => (
            t!("widevine-prompt-terms", version = ""),
            Some(t!("widevine-prompt-installing")),
            None,
            false,
        ),
    };
    let accepting = matches!(state, CdmState::Offered(_));
    let proceed_label = match accepting {
        true => t!("widevine-prompt-accept"),
        false => t!("widevine-prompt-download"),
    };
    let decline_label = match accepting {
        true => t!("widevine-prompt-decline"),
        false => t!("widevine-prompt-later"),
    };

    let mut modal = Modal::new("widevine-prompt", t!("widevine-prompt-title")).detail(detail);
    if let Some(license) = license {
        modal = modal.child(
            text(license.clone())
                .key("widevine-license")
                .role("paragraph")
                .label(license)
                .w_full()
                .padding(theme.metrics.pad)
                .radius(theme.radius)
                .border(BORDER, theme.border.color())
                .fill(theme.secondary.color())
                .font(small)
                .color(theme.muted_foreground.color())
                .wrap(),
        );
    }
    if let Some(note) = note {
        modal = modal.child(
            text(note.clone())
                .key("widevine-note")
                .role("status")
                .label(note)
                .font(small)
                .color(theme.muted_foreground.color())
                .transition(rune::Transition::opacity()),
        );
    }
    modal
        .action(
            Button::new("decline-widevine")
                .ghost()
                .label(decline_label)
                .on_press(decline)
                .build(theme),
        )
        .action(
            Button::new("proceed-widevine")
                .primary()
                .label(proceed_label)
                .disabled(!action)
                .loading(!action)
                .on_press(proceed)
                .build(theme)
                .default_button(),
        )
        .on_dismiss(decline)
        .build(theme, layer);
}
