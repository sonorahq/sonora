use gpui::{App, Context, Menu, MenuItem, OsAction};
use i18n::t;
use input::{
    CloseWindow, DecreaseFontSize, Hide, HideOthers, IncreaseFontSize, MinimizeWindow,
    OpenSettings, Quit, Redo, Refresh, ResetFontSize, ShowAll, SignOut, SongNext, SongPrevious,
    TogglePlayback, ToggleRepeat, ToggleShuffle, Undo, ZoomWindow,
};
use router::Destination;
use state::{AppSettings, Equalizer, Shelf, Sonora};
use ui::{ActiveTheme as _, Copy, Cut, Look, Paste, SelectAll, Theme};

/// How far one font size shortcut moves the size, the same step as the buttons in Settings.
const FONT_STEP: f32 = 1.;

pub fn register(lingers: bool, cx: &mut App) {
    cx.bind_keys(input::bindings());

    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
    cx.on_action(|_: &Hide, cx: &mut App| cx.hide());
    cx.on_action(|_: &HideOthers, cx: &mut App| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx: &mut App| cx.unhide_other_apps());

    cx.on_window_closed(move |cx, _| {
        if !cx.windows().is_empty() {
            return;
        }
        let close_to_tray = Sonora::global(cx).settings.read(cx).close_to_tray();
        match lingers && close_to_tray {
            true => crate::dock::show(false),
            false => cx.quit(),
        }
    })
    .detach();

    cx.on_action(|_: &SignOut, cx: &mut App| {
        let session = Sonora::global(cx).session.clone();
        session.update(cx, |session, cx| session.sign_out(cx));
    });

    // Root handles Refresh whenever a window is open; this runs only without one (the macOS menu).
    cx.on_action(
        |_: &Refresh, cx: &mut App| match router::trail(cx).read(cx).current() {
            Destination::History => {
                let history = Sonora::global(cx).history.clone();
                history.update(cx, |history, cx| history.refresh(cx));
            }
            at if !at.refreshes() => {}
            at => {
                let shelf = match at {
                    Destination::Local(_) => Shelf::Local,
                    Destination::Album(id) | Destination::Playlist(id) => Shelf::of(&id),
                    _ => Shelf::Streaming,
                };
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| library.refresh(shelf, cx));
            }
        },
    );

    cx.on_action(|_: &TogglePlayback, cx: &mut App| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.toggle_play(cx));
    });

    cx.on_action(|_: &SongPrevious, cx: &mut App| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.previous(cx));
    });

    cx.on_action(|_: &SongNext, cx: &mut App| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.next(cx));
    });

    cx.on_action(|_: &ToggleShuffle, cx: &mut App| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.toggle_shuffle(cx));
    });

    cx.on_action(|_: &ToggleRepeat, cx: &mut App| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.toggle_repeat(cx));
    });

    cx.on_action(|_: &Undo, cx: &mut App| edit_equalizer(cx, Equalizer::undo));
    cx.on_action(|_: &Redo, cx: &mut App| edit_equalizer(cx, Equalizer::redo));

    cx.on_action(|_: &IncreaseFontSize, cx: &mut App| {
        change_font_size(cx, |settings, cx| {
            settings.set_font_size(settings.font_size() + FONT_STEP, cx)
        });
    });

    cx.on_action(|_: &DecreaseFontSize, cx: &mut App| {
        change_font_size(cx, |settings, cx| {
            settings.set_font_size(settings.font_size() - FONT_STEP, cx)
        });
    });

    cx.on_action(|_: &ResetFontSize, cx: &mut App| {
        change_font_size(cx, |settings, cx| settings.reset_font_size(cx));
    });

    cx.set_menus(menus());
}

/// The menu bar. Only macOS draws one, so only macOS gets the Edit and Window menus and the
/// application-menu items Cocoa users expect; the other platforms keep the one Sonora menu.
fn menus() -> Vec<Menu> {
    let app = match cfg!(target_os = "macos") {
        true => vec![
            MenuItem::action(t!("app-settings"), OpenSettings),
            MenuItem::separator(),
            MenuItem::action(t!("app-refresh"), Refresh),
            MenuItem::action(t!("app-sign-out"), SignOut),
            MenuItem::separator(),
            MenuItem::action(t!("app-hide"), Hide),
            MenuItem::action(t!("app-hide-others"), HideOthers),
            MenuItem::action(t!("app-show-all"), ShowAll),
            MenuItem::separator(),
            MenuItem::action(t!("app-quit"), Quit),
        ],
        false => vec![
            MenuItem::action(t!("app-refresh"), Refresh),
            MenuItem::action(t!("app-sign-out"), SignOut),
            MenuItem::separator(),
            MenuItem::action(t!("app-quit"), Quit),
        ],
    };
    let mut menus = vec![Menu {
        name: "Sonora".into(),
        disabled: false,
        items: app,
    }];
    if cfg!(target_os = "macos") {
        menus.push(Menu {
            name: t!("app-edit"),
            disabled: false,
            items: vec![
                MenuItem::os_action(t!("app-cut"), Cut, OsAction::Cut),
                MenuItem::os_action(t!("app-copy"), Copy, OsAction::Copy),
                MenuItem::os_action(t!("app-paste"), Paste, OsAction::Paste),
                MenuItem::os_action(t!("app-select-all"), SelectAll, OsAction::SelectAll),
            ],
        });
        menus.push(Menu {
            name: t!("app-window"),
            disabled: false,
            items: vec![
                MenuItem::action(t!("app-close-window"), CloseWindow),
                MenuItem::action(t!("app-minimize"), MinimizeWindow),
                MenuItem::action(t!("app-zoom"), ZoomWindow),
            ],
        });
    }
    menus
}

/// Runs `edit` on the equalizer while its screen is the one shown, the only place undo applies.
fn edit_equalizer(cx: &mut App, edit: fn(&mut Equalizer, &mut Context<Equalizer>)) {
    if router::trail(cx).read(cx).current() == Destination::Equalizer {
        let equalizer = Sonora::global(cx).equalizer.clone();
        equalizer.update(cx, edit);
    }
}

/// Applies `change` to the saved font size and redraws with it, keeping the cover tint the
/// theme already wears so a shortcut does not undo the adaptive palette.
fn change_font_size(
    cx: &mut App,
    change: impl FnOnce(&mut AppSettings, &mut Context<AppSettings>),
) {
    let settings = Sonora::global(cx).settings.clone();
    settings.update(cx, change);
    let (tint, tint_secondary) = (cx.theme().tint, cx.theme().tint_secondary);
    let (look, overrides) = {
        let settings = settings.read(cx);
        (
            Look {
                tint,
                tint_secondary,
                ..settings.look()
            },
            settings.theme_overrides(),
        )
    };
    Theme::set(look, &overrides, cx);
}
