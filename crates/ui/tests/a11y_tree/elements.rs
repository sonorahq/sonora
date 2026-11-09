//! Controls and static elements: buttons, toggles, cards, tabs, fields, sliders and labels.

use rune::accesskit::Action;
use rune::{Cx, El, Tx, col};
use ui::{
    Button, Card, Checkbox, Fact, InfoCard, Input, Kbd, Notice, Radio, Scrubber, Switch, TabBar,
    Toast, TrafficLightControls, Vacancy, WindowControls, spoken, spoken_eyebrow, spoken_heading,
    spoken_vacant,
};

use super::support::*;

fn buttons(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    col()
        .child(
            Button::new("save")
                .label("Save")
                .on_press(|s: &mut App| s.presses += 1)
                .build(&theme),
        )
        .child(
            Button::new("shuffle")
                .icon("icons/shuffle.svg")
                .tooltip("Shuffle")
                .toggled(s.on)
                .on_press(|s: &mut App| s.on = !s.on)
                .build(&theme),
        )
        .child(
            Button::new("off")
                .label("Unavailable")
                .disabled(true)
                .on_press(|s: &mut App| s.presses += 100)
                .build(&theme),
        )
        .child(
            Button::new("picker")
                .label("High")
                .name("Quality")
                .expanded(false)
                .build(&theme),
        )
}

#[test]
fn buttons_read_their_label_tooltip_and_state() {
    let mut rt = mount(App::default(), buttons);
    let tree = rt.accessibility_json();
    let save = node(&tree, "save");
    assert_eq!(said(save), ("button", "Save"));
    assert!(can(save, "click") && can(save, "focus"));
    assert_eq!(
        said(node(&tree, "shuffle")),
        ("button", "Shuffle"),
        "an icon-only button takes its name from its tooltip"
    );
    let off = node(&tree, "off");
    assert!(has(off, "disabled") && !can(off, "click"));
    let picker = node(&tree, "picker");
    assert_eq!(said(picker), ("button", "Quality"));
    assert_eq!(picker["description"], "High");

    assert!(act(&mut rt, "save", Action::Click));
    assert!(!act(&mut rt, "off", Action::Click));
    assert!(act(&mut rt, "shuffle", Action::Click));
    assert_eq!(rt.state.presses, 1);
    assert!(rt.state.on);
}

fn toggles(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    col()
        .child(
            Switch::new("wifi", s.on)
                .name("Wi-Fi")
                .on_press(|s: &mut App| s.on = !s.on)
                .build(&theme),
        )
        .child(
            Checkbox::new("sync", s.on)
                .label("Sync")
                .on_press(|s: &mut App| s.on = !s.on)
                .build(&theme),
        )
        .child(Radio::new("high", s.on).name("High").build(&theme))
}

#[test]
fn switches_checkboxes_and_radios_read_checked() {
    let mut rt = mount(App::default(), toggles);
    let tree = rt.accessibility_json();
    let wifi = node(&tree, "wifi");
    assert_eq!(said(wifi), ("switch", "Wi-Fi"));
    assert!(can(wifi, "click"));
    assert_eq!(said(node(&tree, "sync")), ("check-box", "Sync"));
    assert_eq!(said(node(&tree, "high")), ("radio-button", "High"));
    assert!(act(&mut rt, "wifi", Action::Click));
    assert!(rt.state.on);
    let tree = rt.accessibility_json();
    assert!(has(node(&tree, "wifi"), "checked"));
    assert!(has(node(&tree, "sync"), "checked"));
}

fn cards(_: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    col()
        .child(
            Card::new("album", "Blue")
                .description("Joni Mitchell, 1971")
                .meta("Joni Mitchell", &theme)
                .play(false, |s: &mut App, _: &mut Tx| s.on = true)
                .on_press(|s: &mut App| s.presses += 1)
                .build(&theme),
        )
        .child(
            Card::new("tile", "Hejira")
                .tile(140.)
                .play(false, |s: &mut App, _: &mut Tx| s.on = true)
                .on_press(|s: &mut App| s.presses += 1)
                .build(&theme),
        )
        .child(Card::new("still", "Court and Spark").build(&theme))
}

#[test]
fn cards_read_as_buttons_named_for_their_title() {
    let mut rt = mount(App::default(), cards);
    let tree = rt.accessibility_json();
    let album = node(&tree, "album");
    assert_eq!(said(album), ("button", "Blue"));
    assert_eq!(album["description"], "Joni Mitchell, 1971");
    assert_eq!(said(node(&tree, "still")), ("group", "Court and Spark"));
    assert_eq!(
        with_role(album, "button").len(),
        2,
        "the play button sits in the card"
    );
    assert!(act(&mut rt, "album", Action::Click));
    assert_eq!(rt.state.presses, 1);
    let tile = node(&rt.accessibility_json(), "tile").clone();
    let play = with_role(&tile, "button")
        .into_iter()
        .find(|n| n["key"] == "play")
        .expect("a tile's play button")
        .clone();
    assert!(!play["name"].as_str().unwrap_or("").is_empty());
}

fn tabs(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    TabBar::new("view")
        .name("View")
        .items([
            Button::new("albums")
                .label("Albums")
                .selected(!s.on)
                .on_press(|s: &mut App| s.on = false),
            Button::new("songs")
                .label("Songs")
                .selected(s.on)
                .on_press(|s: &mut App| s.on = true),
        ])
        .build(&theme)
}

#[test]
fn tab_bars_read_as_tab_lists() {
    let mut rt = mount(App::default(), tabs);
    let tree = rt.accessibility_json();
    let list = with_role(&tree, "tab-list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "View");
    assert_eq!(said(node(&tree, "albums")), ("tab", "Albums"));
    assert_eq!(said(node(&tree, "songs")), ("tab", "Songs"));
    assert!(act(&mut rt, "songs", Action::Click));
    assert!(rt.state.on);
}

fn fields(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    col()
        .child(
            Input::new("search", &s.edit, "Search", |s: &mut App| &mut s.edit)
                .icon("icons/search.svg")
                .clearable()
                .build(&theme),
        )
        .child(
            Scrubber::new("volume", &s.scrubber, s.level)
                .name("Volume")
                .value("40%")
                .on_move(|s: &mut App, at: f32, _: &mut Tx| s.level = at)
                .build(&theme),
        )
}

#[test]
fn fields_and_sliders_read_their_names() {
    let mut rt = mount(
        App {
            level: 0.4,
            ..App::default()
        },
        fields,
    );
    let tree = rt.accessibility_json();
    let inputs = with_role(&tree, "text-input");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["name"], "Search");
    let volume = node(&tree, "volume");
    assert_eq!(said(volume), ("slider", "Volume"));
    assert!(can(volume, "increment") && can(volume, "decrement"));
    assert!(act(&mut rt, "volume", Action::Increment));
    assert!((rt.state.level - 0.45).abs() < 1e-3, "{}", rt.state.level);
}

fn statics(_: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    col()
        .child(spoken("line", "Nothing here", &theme))
        .child(spoken_heading("title", "Library", 1, &theme))
        .child(spoken_eyebrow("eyebrow", "Recent", &theme))
        .child(spoken_vacant("empty", "No playlists", &theme))
        .child(Kbd::new("secondary-f").spoken("shortcut").build(&theme))
        .child(
            Notice::new("Offline", "Reconnect to stream")
                .spoken("notice")
                .build(&theme),
        )
        .child(
            Vacancy::new("No songs")
                .detail("Add some")
                .spoken("vacancy")
                .build(&theme),
        )
        .child(
            InfoCard::new("About")
                .spoken("about")
                .child(Fact::new("Year", "1971").spoken("year").build(&theme))
                .build(&theme),
        )
        .child(
            Toast::new("toast", "Added to queue")
                .on_dismiss(|s: &mut App, _: &mut Tx| s.dismissed = true)
                .build(&theme),
        )
        .child(WindowControls::new(false).build::<App>(&theme))
        .child(TrafficLightControls::new(true).build::<App>(&theme))
}

#[test]
fn labels_cards_and_chrome_read_their_names() {
    let mut rt = mount(App::default(), statics);
    let tree = rt.accessibility_json();
    assert_eq!(said(node(&tree, "line")), ("label", "Nothing here"));
    let title = node(&tree, "title");
    assert_eq!(said(title), ("heading", "Library"));
    assert_eq!(said(node(&tree, "eyebrow")), ("label", "Recent"));
    assert_eq!(said(node(&tree, "empty")), ("label", "No playlists"));
    assert!(!node(&tree, "shortcut")["name"].as_str().unwrap().is_empty());
    assert_eq!(said(node(&tree, "notice")), ("group", "Offline"));
    assert_eq!(said(node(&tree, "vacancy")), ("label", "No songs"));
    assert_eq!(said(node(&tree, "about")), ("group", "About"));
    assert_eq!(said(node(&tree, "year")), ("label", "Year"));
    let toast = node(&tree, "toast");
    assert_eq!(said(toast), ("status", "Added to queue"));
    let dismiss = node(toast, "dismiss");
    assert_eq!(dismiss["role"], "button");
    assert!(!dismiss["name"].as_str().unwrap_or("").is_empty());
    for key in ["window-minimize", "window-maximize", "window-close"] {
        let found = with_role(&tree, "button")
            .into_iter()
            .filter(|n| n["key"] == key)
            .count();
        assert_eq!(found, 2, "{key} in both kinds of controls");
    }
    assert!(act(&mut rt, "dismiss", Action::Click));
    assert!(rt.state.dismissed);
}
