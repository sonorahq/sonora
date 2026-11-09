//! Every element on one page, for looking at them: `cargo run -p ui --example gallery -- --snap
//! gallery.png`.

use rune::{AppSpec, Cx, El, TextEdit, Tx, col, row, text};
use ui::*;

struct App {
    on: bool,
    edit: TextEdit,
    code: TextEdit,
    popovers: Popovers,
    sub: bool,
    scrubber: ScrubberState,
    volume: ScrubberState,
    modal: bool,
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let theme = Theme::dark();
    cx.help_look(help_look(&theme));
    let mut layer = Vec::new();
    let buttons = row()
        .gap(8.)
        .center_y()
        .child(Button::new("b1").label("Ghost").icon("icons/play.svg").build(&theme))
        .child(Button::new("b2").label("Secondary").secondary().build(&theme))
        .child(Button::new("b3").label("Outline").outline().build(&theme))
        .child(Button::new("b4").label("Primary").primary().build(&theme))
        .child(Button::new("b5").label("Delete").destructive().build(&theme))
        .child(Button::new("b6").label("Small").small().outline().trailing("icons/chevron-down.svg").build(&theme))
        .child(Button::new("b7").icon("icons/shuffle.svg").tooltip("Shuffle").indicator(true).build(&theme))
        .child(Button::new("b8").label("Off").disabled(true).outline().build(&theme))
        .child(Button::new("b9").label("Loading").loading(true).secondary().build(&theme));
    let toggles = row()
        .gap(16.)
        .center_y()
        .child(Switch::new("s1", true).name("On").build(&theme))
        .child(Switch::new("s2", false).name("Off").build(&theme))
        .child(Checkbox::new("c1", true).label("Checked").build(&theme))
        .child(Checkbox::new("c2", false).label("Unchecked").build(&theme))
        .child(Radio::new("r1", true).build(&theme))
        .child(Radio::new("r2", false).build(&theme))
        .child(Kbd::new("secondary-shift-f").build(&theme))
        .child(Spinner::new().build(&theme))
        .child(ArrowRing::new(Some(0.6)).color(theme.primary).build(&theme))
        .child(ExplicitBadge::new().build(&theme))
        .child(Initials::new("Joni Mitchell", 32.).build(&theme))
        .child(WindowControls::new(false).build::<App>(&theme))
        .child(TrafficLightControls::new(true).build::<App>(&theme));
    let tabs = TabBar::new("tabs").items([
        Button::new("t1").label("Albums").selected(!s.on).on_press(|s: &mut App| s.on = false),
        Button::new("t2").label("Songs").selected(s.on).on_press(|s: &mut App| s.on = true),
        Button::new("t3").label("Artists"),
    ]);
    let picker = Picker::new("quality", &s.popovers, |s: &mut App| &mut s.popovers, "High")
        .item(MenuItem::new("high", "High").selected(true))
        .item(MenuItem::new("low", "Low"))
        .build(&theme, &mut layer);
    let sub = Menu::new("more").item(MenuItem::new("a", "Playlist A").icon("icons/list.svg")).item(MenuItem::new("b", "Playlist B"));
    let menu = Menu::new("file")
        .item(MenuItem::new("open", "Play next").icon("icons/play.svg"))
        .item(MenuItem::new("wrap", "Shuffle").checked(true))
        .item(MenuItem::separator("rule"))
        .item(MenuItem::new("more", "Add to playlist").submenu(sub, s.sub, |s: &mut App, o| s.sub = o));
    let popover = Popover::new("file", &s.popovers, |s: &mut App| &mut s.popovers)
        .button(Button::new("file-b").label("Menu").outline())
        .menu(menu)
        .build(&theme, &mut layer);
    let cards = row()
        .gap(16.)
        .child(
            col().w(320.)
                .child(Card::new("card1", "Blue").meta("Joni Mitchell", &theme).play(false, |_: &mut App, _: &mut Tx| {}).on_press(|_: &mut App| {}).build(&theme))
                .child(Card::new("card2", "Hejira").meta("Joni Mitchell · 1976", &theme).explicit().chosen(true).on_press(|_: &mut App| {}).build(&theme))
                .child(Card::skeleton("card3").build(&theme)),
        )
        .child(Card::new("tile", "Court and Spark").tile(140.).meta("Joni Mitchell", &theme).play(true, |_: &mut App, _: &mut Tx| {}).on_press(|_: &mut App| {}).build(&theme))
        .child(
            col().w(220.).gap(8.)
                .child(OptionCard::new("o1").label("Dark").selected(true).on_press(|_: &mut App| {}).build(&theme, &mut layer))
                .child(OptionCard::new("o2").label("Add").dashed().on_press(|_: &mut App| {}).build(&theme, &mut layer)),
        );
    let fields = row()
        .gap(16.)
        .w(900.)
        .center_y()
        .child(col().w(260.).child(Input::new("search", &s.edit, "Search", |s: &mut App| &mut s.edit).icon("icons/search.svg").shortcut("secondary-f").build(&theme)))
        .child(col().w(260.).gap(8.)
            .child(Scrubber::live("seek", &s.scrubber).name("Seek").on_move(|_: &mut App, _, _: &mut Tx| {}).build(&theme))
            .child(Scrubber::new("vol", &s.volume, 0.7).name("Volume").bubble(0.7, "70%").build(&theme))
            .child(LevelMeter::new(-6.).peak(-3.).build(&theme)))
        .child(OtpInput::new("otp", &s.code, 6, "Code", |s: &mut App| &mut s.code).grouped(3).build(&theme))
        .child(Qr::new("https://sonora.app").size(96.).rounded(8.).build());
    let notes = row()
        .gap(16.)
        .items_start()
        .child(col().w(300.).child(Notice::new("Offline", "Reconnect to stream music.").failed().build(&theme)))
        .child(col().w(360.).child(Toast::new("toast", "Added Blue to the queue").strong("Blue").detail("3 songs").action("Undo", |_: &mut App, _: &mut Tx| {}).on_dismiss(|_: &mut App, _: &mut Tx| {}).build(&theme)))
        .child(col().w(260.).child(Vacancy::new("No playlists").detail("Make one to see it here").icon("icons/list.svg").build(&theme)))
        .child(col().w(260.).child(InfoCard::new("About").child(Fact::new("Year", "1971").build(&theme)).child(Fact::new("Label", "Reprise").striped(true).build(&theme)).build(&theme)));
    if s.modal {
        Modal::new("dialog", "Delete playlist?")
            .detail("This cannot be undone.")
            .child(text("Road trip").color(theme.foreground.color()))
            .action(Button::new("cancel").label("Cancel").outline().build(&theme))
            .action(Button::new("delete").label("Delete").destructive().build(&theme))
            .close_button()
            .on_dismiss(|s: &mut App| s.modal = false)
            .build(&theme, &mut layer);
    }
    let page = col()
        .padding(24.)
        .gap(20.)
        .fill(theme.background.color())
        .child(spoken_heading("h", "Gallery", 1, &theme))
        .child(eyebrow("Buttons", &theme))
        .child(buttons)
        .child(toggles)
        .child(row().gap(16.).center_y().child(tabs.build(&theme)).child(picker).child(popover))
        .child(cards)
        .child(fields)
        .child(notes);
    root(page, layer)
}

fn init() -> App {
    let menu = std::env::var("GALLERY_MENU").is_ok();
    let scrubber = ScrubberState::new();
    scrubber.set(0.35);
    let mut popovers = Popovers::default();
    if menu {
        popovers.open("file");
    }
    App {
        on: false,
        edit: TextEdit::new(""),
        code: TextEdit::new("123"),
        popovers,
        sub: menu,
        scrubber,
        volume: ScrubberState::new(),
        modal: std::env::var("GALLERY_MODAL").is_ok(),
    }
}

fn main() {
    rune::run(AppSpec::new("Gallery", (1400., 1000.), init, view));
}
