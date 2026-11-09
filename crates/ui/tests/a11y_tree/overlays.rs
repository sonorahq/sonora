//! Floating surfaces: a popover's menu with checks and a submenu, a context menu, a picker, a
//! searchable menu and a dialog.

use rune::accesskit::Action;
use rune::{Cx, El, Tx, col, text};
use ui::{Button, Menu, MenuItem, Modal, Picker, Popover, Popup, root};

use super::support::*;

fn popover(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    let mut layer = Vec::new();
    let sub = Menu::new("more")
        .item(MenuItem::new("deep", "Deep").on_press(|s: &mut App| s.presses += 10));
    let menu = Menu::new("file")
        .name("File")
        .item(MenuItem::new("open", "Open").on_press(|s: &mut App| s.presses += 1))
        .item(MenuItem::separator("rule"))
        .item(
            MenuItem::new("wrap", "Wrap lines")
                .checked(s.on)
                .stays_open()
                .on_press(|s: &mut App| s.on = !s.on),
        )
        .item(MenuItem::new("more", "More").submenu(sub, s.sub, |s: &mut App, open| s.sub = open));
    let trigger = Popover::new("file", &s.popovers, |s: &mut App| &mut s.popovers)
        .button(Button::new("file-button").label("File"))
        .menu(menu)
        .commands()
        .build(&theme, &mut layer);
    root(col().padding(40.).child(trigger), layer)
}

#[test]
fn a_popover_opens_a_menu_of_named_items() {
    let mut rt = mount(App::default(), popover);
    let tree = rt.accessibility_json();
    let button = node(&tree, "file-button");
    assert_eq!(said(button), ("button", "File"));
    assert!(has(button, "collapsed"));
    assert!(with_role(&tree, "menu").is_empty());

    assert!(act(&mut rt, "file-button", Action::Click));
    let tree = rt.accessibility_json();
    let menus = with_role(&tree, "menu");
    assert_eq!(menus.len(), 1, "the menu opens");
    assert_eq!(menus[0]["name"], "File");
    assert_eq!(said(node(&tree, "file-open")), ("menu-item", "Open"));
    assert_eq!(
        said(node(&tree, "file-wrap")),
        ("menu-item-check-box", "Wrap lines")
    );
    assert_eq!(said(node(&tree, "file-more")), ("menu-item", "More"));

    assert!(act(&mut rt, "file-wrap", Action::Click));
    assert!(rt.state.on);
    assert!(
        rt.state.popovers.any(),
        "a row that stays open keeps the menu"
    );

    rt.state.sub = true;
    settle(&mut rt);
    let tree = rt.accessibility_json();
    assert_eq!(said(node(&tree, "more-deep")), ("menu-item", "Deep"));
    assert!(act(&mut rt, "more-deep", Action::Click));
    assert_eq!(rt.state.presses, 10);
    assert!(!rt.state.popovers.any(), "a command closes the menu");
}

fn context(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    let mut layer = Vec::new();
    if !s.dismissed {
        Popup::new(
            "row-menu",
            (120., 80.),
            Menu::new("row").name("Song").item(
                MenuItem::new("queue", "Add to queue").on_press(|s: &mut App| s.presses += 1),
            ),
        )
        .on_close(|s: &mut App| s.dismissed = true)
        .build(&theme, &mut layer);
    }
    root(col().child(text("Songs")), layer)
}

#[test]
fn a_context_menu_opens_at_a_point_and_closes_on_a_pick() {
    let mut rt = mount(App::default(), context);
    let tree = rt.accessibility_json();
    assert_eq!(said(node(&tree, "row-menu")), ("menu", "Song"));
    assert_eq!(
        said(node(&tree, "row-queue")),
        ("menu-item", "Add to queue")
    );
    assert!(act(&mut rt, "row-queue", Action::Click));
    assert_eq!(rt.state.presses, 1);
    assert!(rt.state.dismissed);
}

fn picker(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    let mut layer = Vec::new();
    let trigger = Picker::new(
        "quality",
        &s.popovers,
        |s: &mut App| &mut s.popovers,
        "High",
    )
    .name("Quality")
    .item(
        MenuItem::new("high", "High")
            .selected(!s.on)
            .on_press(|s: &mut App| s.on = false),
    )
    .item(
        MenuItem::new("low", "Low")
            .selected(s.on)
            .on_press(|s: &mut App| s.on = true),
    )
    .build(&theme, &mut layer);
    root(col().padding(40.).child(trigger), layer)
}

#[test]
fn a_picker_reads_its_setting_and_its_choice() {
    let mut rt = mount(App::default(), picker);
    let tree = rt.accessibility_json();
    let trigger = node(&tree, "quality-picker");
    assert_eq!(said(trigger), ("button", "Quality"));
    assert_eq!(trigger["description"], "High");
    assert!(act(&mut rt, "quality-picker", Action::Click));
    let tree = rt.accessibility_json();
    assert_eq!(said(node(&tree, "quality-menu-low")), ("menu-item", "Low"));
    assert!(act(&mut rt, "quality-menu-low", Action::Click));
    assert!(rt.state.on);
    assert!(!rt.state.popovers.any());
}

fn searchable(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    let mut layer = Vec::new();
    let query = s.search.query();
    let items = ["Jazz", "Folk", "Blues"]
        .into_iter()
        .filter(|name| name.to_lowercase().contains(&query))
        .map(|name| {
            MenuItem::new(name.to_lowercase(), name).on_press(|s: &mut App| s.presses += 1)
        });
    Menu::new("genres")
        .name("Genres")
        .search(&s.search, "Search genres", |s: &mut App| &mut s.search)
        .items(items)
        .on_dismiss(|s: &mut App| s.dismissed = true)
        .anchor("anchor", ui::Placement::BottomStart, 4.)
        .build(&theme, &mut layer);
    root(
        col().padding(40.).child(text("Genres").key("anchor")),
        layer,
    )
}

#[test]
fn a_menu_search_is_a_named_field_over_the_rows() {
    let rt = mount(App::default(), searchable);
    let tree = rt.accessibility_json();
    let menu = node(&tree, "genres-menu");
    assert_eq!(said(menu), ("menu", "Genres"));
    let fields = with_role(menu, "text-input");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["name"], "Search genres");
    assert_eq!(with_role(menu, "menu-item").len(), 3);
}

fn dialog(s: &App, _: &mut Cx<App>) -> El {
    let theme = theme();
    let mut layer = Vec::new();
    if !s.dismissed {
        Modal::new("delete", "Delete playlist?")
            .detail("This cannot be undone")
            .child(text("Road trip"))
            .action(
                Button::new("confirm")
                    .label("Delete")
                    .destructive()
                    .on_press_tx(|s: &mut App, _: &mut Tx| s.presses += 1)
                    .build(&theme),
            )
            .close_button()
            .on_dismiss(|s: &mut App| s.dismissed = true)
            .build(&theme, &mut layer);
    }
    root(col().child(text("Library")), layer)
}

#[test]
fn a_dialog_reads_its_title_and_holds_the_tree() {
    let mut rt = mount(App::default(), dialog);
    let tree = rt.accessibility_json();
    let panel = node(&tree, "delete-dialog");
    assert_eq!(said(panel), ("dialog", "Delete playlist?"));
    assert_eq!(panel["description"], "This cannot be undone");
    assert_eq!(said(node(panel, "confirm")), ("button", "Delete"));
    let close = node(panel, "close");
    assert_eq!(close["role"], "button");
    assert!(!close["name"].as_str().unwrap_or("").is_empty());
    assert!(act(&mut rt, "close", Action::Click));
    assert!(rt.state.dismissed);
}
