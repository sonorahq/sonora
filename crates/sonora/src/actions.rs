use gpui::{App, Menu, MenuItem};
use i18n::t;
use input::{
    Quit, RefreshLibrary, SeekBack, SeekForward, SignOut, SongNext, SongPrevious, TogglePlayback,
};
use router::Destination;
use state::Sonora;

pub fn register(lingers: bool, cx: &mut App) {
    cx.bind_keys(input::bindings());

    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

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

    cx.on_action(|_: &SeekBack, cx: &mut App| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.seek_back(cx));
    });

    cx.on_action(|_: &SeekForward, cx: &mut App| {
        let playback = Sonora::global(cx).playback.clone();
        playback.update(cx, |playback, cx| playback.seek_forward(cx));
    });

    cx.set_menus(vec![Menu {
        name: "Sonora".into(),
        disabled: false,
        items: vec![
            MenuItem::action(t!("app-refresh-library"), RefreshLibrary),
            MenuItem::action(t!("app-sign-out"), SignOut),
            MenuItem::separator(),
            MenuItem::action(t!("app-quit"), Quit),
        ],
    }]);
}
