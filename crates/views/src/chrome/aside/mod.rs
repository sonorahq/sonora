//! The panels of the right sidebar: the queue, the lyrics and the members of a Session or Jam.
//! Each draws under the same header, and switching tabs cross-fades one into the next.

pub(crate) mod lyrics;
pub(crate) mod members;
pub(crate) mod queue;

use i18n::t;
use rune::{Cx, El, Transition, col, row};
use state::SideTab;
use ui::Theme;

use crate::Ui;

/// The sidebar's content for `tab`, the panel keyed by its tab so a switch fades between them.
pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>, tab: SideTab) -> El {
    let panel = match tab {
        SideTab::Queue => queue::view(ui, cx, layer),
        SideTab::Lyrics => lyrics::view(ui, cx, layer),
        SideTab::Members => members::view(ui, cx, layer),
    };
    col().key("aside-stack").grow(1.).min_h(0.).min_w(0.).child(
        panel
            .key(format!("aside-{}", tab_id(tab)))
            .role("complementary")
            .label(title(tab))
            .absolute(0., 0.)
            .w_full()
            .h_full()
            .transition(Transition::opacity()),
    )
}

/// The panel's name, read in the language shown.
pub(crate) fn title(tab: SideTab) -> String {
    match tab {
        SideTab::Queue => t!("queue-title"),
        SideTab::Lyrics => t!("lyrics-title"),
        SideTab::Members => t!("members-title"),
    }
    .to_string()
}

/// The strip across a panel's top: its title in capitals over a hairline, with `actions` at its
/// end.
pub(crate) fn header(tab: SideTab, actions: Option<El>, theme: &Theme) -> El {
    row()
        .key(format!("aside-header-{}", tab_id(tab)))
        .w_full()
        .h(theme.metrics.header.round())
        .center_y()
        .justify_between()
        .gap(theme.metrics.pad)
        .px(theme.metrics.pad)
        .border_sides([0., 0., 1., 0.], theme.border.color())
        .child(ui::eyebrow(title(tab), theme))
        .children(actions)
}

fn tab_id(tab: SideTab) -> &'static str {
    match tab {
        SideTab::Queue => "queue",
        SideTab::Lyrics => "lyrics",
        SideTab::Members => "members",
    }
}
