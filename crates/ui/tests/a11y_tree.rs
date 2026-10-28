//! Headless regression suite keeping sonorahq/sonora#1101 fixed (NVDA heard silence).
//!
//! A node reaches the AccessKit tree only when its element carries BOTH an id
//! and a role (`Element::a11y_role()` returns `Some`). Each test renders one
//! primitive inside a headless window and snapshots the built element, which is
//! exactly what the frame builder inspects before pushing a node. Add a case
//! here with every new primitive so CI fails when a role or name goes missing.
//!
//! A full `TreeUpdate` dump is not possible here: `TestWindow::a11y_init` is a
//! no-op, so the active flag never flips and `debug_a11y_tree_json` stays
//! `None`. The `NO NODE` lines below are the silence: no role means no node,
//! so a screen reader falls back to announcing the whole window.
//!
//! Switch has no case on purpose: its render boxes the element into
//! `AnyElement`/`AnimationElement` before returning, and neither forwards
//! `a11y_role`, so the built output always reads `role=None` even once the
//! inner div carries a role (the real tree still nests the inner node during
//! prepaint). TabBar, Menu, Modal and Table are out for the same reason: the
//! TabList, Menu, MenuItem, Dialog and Row roles sit on elements nested below
//! what `render` returns, so only a live tree sees them, and the test platform
//! drops the `a11y_init` callbacks, so no headless run ever builds one. Those
//! stay manual-only NVDA passes. `Input` is covered below through a real
//! entity, since its field div carries the role itself.

use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, AppContext as _, Context, Element, IntoElement, Render, RenderOnce, TestAppContext,
    Window,
    accesskit::{Action, Node},
    div,
};
use ui::{Button, Checkbox, Input, Radio, Scrubber, ScrubberState, Theme, Toast};

/// Formats what the frame builder would put in the tree for `element`.
fn snapshot(element_name: &str, element: &impl Element) -> String {
    let id = element.id().map(|id| format!("{id:?}"));
    match element.a11y_role() {
        None => format!("{element_name}: NO NODE (id={id:?}, role=None)"),
        Some(role) => {
            let mut node = Node::new(role);
            element.write_a11y_info(&mut node);
            format!(
                "{element_name}: role={role:?} id={id:?} name={:?} click={} focus={} toggled={:?} value={:?}",
                node.label(),
                node.supports_action(Action::Click),
                node.supports_action(Action::Focus),
                node.toggled(),
                node.numeric_value(),
            )
        }
    }
}

type Inspect = Box<dyn FnOnce(&mut Window, &mut App) -> String + 'static>;

/// Root view that builds one primitive, snapshots it, then draws nothing.
struct Probe {
    log: Rc<RefCell<Vec<String>>>,
    inspect: Option<Inspect>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(inspect) = self.inspect.take() {
            self.log.borrow_mut().push(inspect(window, cx));
        }
        div().into_any_element()
    }
}

/// Opens a headless window whose root builds one primitive and returns the
/// snapshot lines it recorded.
fn open_probe(
    cx: &mut TestAppContext,
    inspect: impl FnOnce(&mut Window, &mut App) -> String + 'static,
) -> Vec<String> {
    cx.update(|cx| cx.set_global(Theme::dark()));
    let log = Rc::new(RefCell::new(Vec::new()));
    let probe_log = log.clone();
    let (_view, probe_cx) = cx.add_window_view(move |_, _| Probe {
        log: probe_log,
        inspect: Some(Box::new(inspect)),
    });
    probe_cx.run_until_parked();
    cx.run_until_parked();
    let lines = log.borrow().clone();
    for line in &lines {
        println!("{line}");
    }
    lines
}

fn rendered(lines: &[String], what: &str) -> Vec<String> {
    assert!(
        !lines.is_empty(),
        "{what}: probe view never rendered, no snapshot to assert on"
    );
    lines.to_vec()
}

#[gpui::test]
async fn button_advertises_role_and_name(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Button::new("play")
            .label("Play")
            .render(window, cx)
            .into_element();
        snapshot("button", &element)
    });
    let lines = rendered(&lines, "button");
    assert!(
        lines.iter().any(|line| line.contains("role=Button")),
        "button exposes no Role::Button, screen readers skip it: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Play")),
        "button exposes no accessible name, screen readers announce nothing: {lines:?}"
    );
}

#[gpui::test]
async fn icon_only_button_names_itself_from_tooltip(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Button::new("play")
            .icon("icons/play.svg")
            .tooltip("play")
            .render(window, cx)
            .into_element();
        snapshot("icon-button", &element)
    });
    let lines = rendered(&lines, "icon-button");
    assert!(
        lines.iter().any(|line| line.contains("role=Button")),
        "icon-only button exposes no Role::Button, screen readers skip it: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("name=Some")),
        "icon-only button exposes no accessible name for its tooltip: {lines:?}"
    );
}

#[gpui::test]
async fn checkbox_advertises_role_and_state(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Checkbox::new("shuffle", true)
            .label("Shuffle")
            .render(window, cx)
            .into_element();
        snapshot("checkbox", &element)
    });
    let lines = rendered(&lines, "checkbox");
    assert!(
        lines.iter().any(|line| line.contains("role=CheckBox")),
        "checkbox exposes no Role::CheckBox, screen readers skip it: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("toggled=Some")),
        "checkbox exposes no toggled state, screen readers cannot announce on/off: {lines:?}"
    );
}

#[gpui::test]
async fn scrubber_advertises_role_and_value(cx: &mut TestAppContext) {
    let state = ScrubberState::new("volume");
    let lines = open_probe(cx, move |window, cx| {
        let element = Scrubber::new(&state, 0.5).render(window, cx).into_element();
        snapshot("scrubber", &element)
    });
    let lines = rendered(&lines, "scrubber");
    assert!(
        lines.iter().any(|line| line.contains("role=Slider")),
        "scrubber exposes no Role::Slider, screen readers skip it: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("0.5")),
        "scrubber exposes no numeric value, screen readers cannot announce position: {lines:?}"
    );
}

/// Keeps the linter honest that every snapshot goes through one formatter.
#[test]
fn snapshot_marks_missing_role_as_no_node() {
    let element = div().into_element();
    assert!(snapshot("plain-div", &element).contains("NO NODE"));
}

#[gpui::test]
async fn radio_advertises_role_and_state(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Radio::new("quality", true)
            .render(window, cx)
            .into_element();
        snapshot("radio", &element)
    });
    let lines = rendered(&lines, "radio");
    assert!(
        lines.iter().any(|line| line.contains("role=RadioButton")),
        "radio exposes no Role::RadioButton, screen readers skip it: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("toggled=Some")),
        "radio exposes no toggled state, screen readers cannot announce on/off: {lines:?}"
    );
}

#[gpui::test]
async fn input_names_itself_from_hint(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let field = cx.new(|cx| Input::new("common-search", cx));
        field.update(cx, |field, cx| {
            snapshot("input", &field.render(window, cx).into_element())
        })
    });
    let lines = rendered(&lines, "input");
    assert!(
        lines.iter().any(|line| line.contains("role=TextInput")),
        "input exposes no Role::TextInput, screen readers skip it: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("name=Some")),
        "input exposes no accessible name for its hint, screen readers announce nothing: {lines:?}"
    );
}

#[gpui::test]
async fn toast_announces_status_with_message(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Toast::new("saved", "Playlist saved")
            .render(window, cx)
            .into_element();
        snapshot("toast", &element)
    });
    let lines = rendered(&lines, "toast");
    assert!(
        lines.iter().any(|line| line.contains("role=Status")),
        "toast exposes no Role::Status, screen readers skip it: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("Playlist saved")),
        "toast exposes no accessible name for its message, screen readers announce nothing: {lines:?}"
    );
}
