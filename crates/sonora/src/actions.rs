use gpui::{App, Menu, MenuItem, OsAction};
use i18n::t;
use input::{
    CloseWindow, Hide, HideOthers, MinimizeWindow, OpenSettings, Quit, RefreshLibrary, ShowAll,
    SignOut, SongNext, SongPrevious, TogglePlayback, ZoomWindow,
};
use router::Destination;
use state::Sonora;
use ui::{Copy, Cut, Paste, SelectAll};

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

    cx.on_action(
        |_: &RefreshLibrary, cx: &mut App| match router::trail(cx).read(cx).current() {
            Destination::History => {
                let history = Sonora::global(cx).history.clone();
                history.update(cx, |history, cx| history.refresh(cx));
            }
            _ => {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| library.refresh(cx));
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

    cx.set_menus(menus());
}

/// The menu bar. Only macOS draws one, so only macOS gets the Edit and Window menus and the
/// application-menu items Cocoa users expect; the other platforms keep the one Sonora menu.
fn menus() -> Vec<Menu> {
    let app = match cfg!(target_os = "macos") {
        true => vec![
            MenuItem::action(t!("app-settings"), OpenSettings),
            MenuItem::separator(),
            MenuItem::action(t!("app-refresh-library"), RefreshLibrary),
            MenuItem::action(t!("app-sign-out"), SignOut),
            MenuItem::separator(),
            MenuItem::action(t!("app-hide"), Hide),
            MenuItem::action(t!("app-hide-others"), HideOthers),
            MenuItem::action(t!("app-show-all"), ShowAll),
            MenuItem::separator(),
            MenuItem::action(t!("app-quit"), Quit),
        ],
        false => vec![
            MenuItem::action(t!("app-refresh-library"), RefreshLibrary),
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
