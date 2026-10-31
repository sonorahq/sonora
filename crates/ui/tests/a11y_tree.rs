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
//!
//! The same goes for the nodes nested below a primitive's own: the named
//! trigger of a `Popover` face, the Tab buttons of a `TabBar`, the positions of
//! menu items, the Link and Label items of `InlineLinks`, the two Slider
//! handles of a `RangeScrubber`, the window control and rail arrow buttons, and
//! the sort state and focused row of a `Table`. `InfoCard`, `Fact`, `Notice`,
//! `Vacancy` and `Kbd` return an `AnyElement` once `spoken` gives them an id,
//! so they are manual passes too.

use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, AppContext as _, Context, Element, IntoElement, Render, RenderOnce, TestAppContext,
    Window,
    accesskit::{Action, Node, Role},
    div,
};
use ui::{
    Button, Card, Checkbox, Input, OptionCard, Radio, RangeScrubber, RangeState, Scrubber,
    ScrubberState, Theme, Toast,
};

/// Formats what the frame builder would put in the tree for `element`.
fn snapshot(element_name: &str, element: &impl Element) -> String {
    let id = element.id().map(|id| format!("{id:?}"));
    match element.a11y_role() {
        None => format!("{element_name}: NO NODE (id={id:?}, role=None)"),
        Some(role) => {
            let mut node = Node::new(role);
            element.write_a11y_info(&mut node);
            // AccessKit names a label by its value on every platform and ignores its aria label.
            let name = match role {
                Role::Label => node.value(),
                _ => node.label(),
            };
            format!(
                "{element_name}: role={role:?} id={id:?} name={name:?} description={:?} click={} focus={} toggled={:?} value={:?} text={:?} placeholder={:?} selected={:?} expanded={:?} level={:?} step={}",
                node.description(),
                node.supports_action(Action::Click),
                node.supports_action(Action::Focus),
                node.toggled(),
                node.numeric_value(),
                node.value(),
                node.placeholder(),
                node.is_selected(),
                node.is_expanded(),
                node.level(),
                node.supports_action(Action::Increment) && node.supports_action(Action::Decrement),
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
async fn named_button_reads_its_label_as_description(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Button::new("language-picker")
            .label("English")
            .name("Language")
            .render(window, cx)
            .into_element();
        snapshot("named-button", &element)
    });
    let lines = rendered(&lines, "named-button");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"Language\")")),
        "named button is announced by its visible label instead of its name: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("description=Some(\"English\")")),
        "named button drops its visible label, the current choice goes unheard: {lines:?}"
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

#[gpui::test]
async fn spoken_text_names_itself_from_its_value(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |_, _| {
        let element = ui::spoken("version", "0.45.0").into_element();
        snapshot("spoken", &element)
    });
    let lines = rendered(&lines, "spoken");
    assert!(
        lines.iter().any(|line| line.contains("role=Label")),
        "spoken text exposes no Role::Label, screen readers skip it: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"0.45.0\")")),
        "spoken text has no value, a label without one is announced as nothing: {lines:?}"
    );
}

#[gpui::test]
async fn checkbox_takes_a_name_without_a_label(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Checkbox::new("consent", false)
            .name("Share usage")
            .render(window, cx)
            .into_element();
        snapshot("named-checkbox", &element)
    });
    let lines = rendered(&lines, "named-checkbox");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"Share usage\")")),
        "checkbox without a label is announced as nothing but its state: {lines:?}"
    );
}

#[gpui::test]
async fn radio_names_the_option_it_marks(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Radio::new("quality-high", true)
            .name("High")
            .render(window, cx)
            .into_element();
        snapshot("named-radio", &element)
    });
    let lines = rendered(&lines, "named-radio");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"High\")")),
        "radio mark is announced as nothing but its state: {lines:?}"
    );
}

#[gpui::test]
async fn tab_button_reads_as_a_selected_tab(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Button::new("songs")
            .label("Songs")
            .tab()
            .selected(true)
            .render(window, cx)
            .into_element();
        snapshot("tab", &element)
    });
    let lines = rendered(&lines, "tab");
    assert!(
        lines.iter().any(|line| line.contains("role=Tab")),
        "tab button reads as a plain button, the tab list loses its shape: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("selected=Some(true)")),
        "tab button does not say which tab is chosen: {lines:?}"
    );
}

#[gpui::test]
async fn button_advertises_expanded_and_toggled_state(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Button::new("shuffle")
            .label("Shuffle")
            .toggled(true)
            .expanded(false)
            .render(window, cx)
            .into_element();
        snapshot("state-button", &element)
    });
    let lines = rendered(&lines, "state-button");
    assert!(
        lines.iter().any(|line| line.contains("toggled=Some(True)")),
        "toggle button does not say whether it is on: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("expanded=Some(false)")),
        "menu button does not say whether its menu is open: {lines:?}"
    );
}

#[gpui::test]
async fn option_card_reads_as_a_named_radio(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = OptionCard::new("theme-dark")
            .label("Dark")
            .detail("Easy on the eyes")
            .selected(true)
            .on_click(|_, _, _| {})
            .render(window, cx)
            .into_element();
        snapshot("option-card", &element)
    });
    let lines = rendered(&lines, "option-card");
    assert!(
        lines.iter().any(|line| line.contains("role=RadioButton")),
        "option card exposes no Role::RadioButton, screen readers skip it: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"Dark\")")),
        "option card exposes no accessible name for its label: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("toggled=Some(True)")),
        "option card does not say which option is chosen: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("description=Some(\"Easy on the eyes\")")),
        "option card drops its detail line: {lines:?}"
    );
}

#[gpui::test]
async fn card_reads_its_description_and_choice(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Card::new("album", "Abbey Road")
            .description("The Beatles")
            .chosen(true)
            .press(|_, _, _| {})
            .render(window, cx)
            .into_element();
        snapshot("card", &element)
    });
    let lines = rendered(&lines, "card");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"Abbey Road\")")),
        "card exposes no accessible name for its title: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("description=Some(\"The Beatles\")")),
        "card drops what its meta line says: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("selected=Some(true)")),
        "card does not say it is the chosen one: {lines:?}"
    );
}

#[gpui::test]
async fn scrubber_names_itself_and_steps(cx: &mut TestAppContext) {
    let state = ScrubberState::new("volume");
    let lines = open_probe(cx, move |window, cx| {
        let element = Scrubber::new(&state, 0.5)
            .name("Volume")
            .value("50%")
            .on_move(|_, _, _| {})
            .render(window, cx)
            .into_element();
        snapshot("named-scrubber", &element)
    });
    let lines = rendered(&lines, "named-scrubber");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"Volume\")")),
        "scrubber exposes no accessible name: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("text=Some(\"50%\")")),
        "scrubber reads a bare fraction instead of its value: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("step=true")),
        "scrubber cannot be stepped from a screen reader: {lines:?}"
    );
}

#[gpui::test]
async fn range_scrubber_groups_its_handles(cx: &mut TestAppContext) {
    let state = RangeState::new("year");
    let lines = open_probe(cx, move |window, cx| {
        let element = RangeScrubber::new(&state, (0.2, 0.8))
            .name("Year")
            .render(window, cx)
            .into_element();
        snapshot("range", &element)
    });
    let lines = rendered(&lines, "range");
    assert!(
        lines.iter().any(|line| line.contains("role=Group")),
        "range scrubber exposes no node around its handles: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"Year\")")),
        "range scrubber does not say what it narrows: {lines:?}"
    );
}

#[gpui::test]
async fn masked_input_reads_as_a_password_field(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let field = cx.new(|cx| Input::new("common-search", cx).masked());
        field.update(cx, |field, cx| {
            snapshot("masked-input", &field.render(window, cx).into_element())
        })
    });
    let lines = rendered(&lines, "masked-input");
    assert!(
        lines.iter().any(|line| line.contains("role=PasswordInput")),
        "masked input does not say it hides what is typed: {lines:?}"
    );
}

#[gpui::test]
async fn named_input_reads_its_hint_as_placeholder(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let field = cx.new(|cx| Input::new("common-search", cx).name("common-save"));
        field.update(cx, |field, cx| {
            snapshot("named-input", &field.render(window, cx).into_element())
        })
    });
    let lines = rendered(&lines, "named-input");
    assert!(
        lines.iter().any(|line| line.contains("placeholder=Some")),
        "named input drops its hint: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("name=None")),
        "named input exposes no accessible name: {lines:?}"
    );
}

#[gpui::test]
async fn spoken_heading_lists_among_headings(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |_, cx| {
        let element = ui::spoken_heading("about", "About", 2, cx).into_element();
        snapshot("spoken-heading", &element)
    });
    let lines = rendered(&lines, "spoken-heading");
    assert!(
        lines.iter().any(|line| line.contains("role=Heading")),
        "spoken heading exposes no Role::Heading: {lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"About\")")),
        "spoken heading exposes no accessible name: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("level=Some(2)")),
        "spoken heading has no level: {lines:?}"
    );
}

#[gpui::test]
async fn spoken_eyebrow_reads_as_written(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |_, cx| {
        let element = ui::spoken_eyebrow("pinned", "Pinned", cx).into_element();
        snapshot("spoken-eyebrow", &element)
    });
    let lines = rendered(&lines, "spoken-eyebrow");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"Pinned\")")),
        "spoken eyebrow is read in capitals or not at all: {lines:?}"
    );
}

#[gpui::test]
async fn spoken_vacant_names_itself_from_its_value(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |_, cx| {
        let element = ui::spoken_vacant("empty", "Nothing here yet", cx).into_element();
        snapshot("spoken-vacant", &element)
    });
    let lines = rendered(&lines, "spoken-vacant");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("name=Some(\"Nothing here yet\")")),
        "spoken vacancy is announced as nothing: {lines:?}"
    );
}

#[gpui::test]
async fn silent_radio_stays_out_of_the_tree(cx: &mut TestAppContext) {
    let lines = open_probe(cx, |window, cx| {
        let element = Radio::new("provider-radio", true)
            .silent()
            .render(window, cx)
            .into_element();
        snapshot("silent-radio", &element)
    });
    let lines = rendered(&lines, "silent-radio");
    assert!(
        lines.iter().all(|line| line.contains("NO NODE")),
        "a silent radio still adds a nameless radio button beside the row that names it: {lines:?}"
    );
}
