use gpui::{AnyView, Context, Entity, MouseButton, NavigationDirection, Render, Task};
use gpui::{App, Font, FontFallbacks, SharedString, StyleRefinement, font, prelude::*};
use gpui::{Window, div};
use input::{
    CloseWindow, MinimizeWindow, NavigateBack, NavigateForward, OpenFilter, OpenSearch,
    OpenSettings, Refresh, ToggleFullscreen, ToggleLyrics, ToggleQueue, ToggleWindowFullscreen,
    ZoomWindow,
};
use router::{Destination, NavigationEvent, Screen, back, forward, navigate};
use state::{
    ArtistDetail, Detail, GenreDetails, Genres, HistoryState, Home, Io, Library, LibraryState,
    Network, Playback, Profile, Queue, Reconnected, SYSTEM_FONT, Scan, Search, Session,
    SessionEvent, SessionState, Shelf, SideTab, SongDetail, Sonora,
};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use ui::WindowFrame;
use ui::{
    ActiveTheme as _, Dismiss, Entrance, INPUT_CONTEXT, Look, Stillness, Theme, ThemeKind,
    clear_listing,
};

use crate::chrome::{
    Busy, JamInvite, ThemeInvite, TitleBar, TitleBarEvent, TitleBarOptions, Toolbar, Tooled,
};
use crate::screens::search::SearchView;
use crate::screens::settings::SettingsHeader;
use crate::shared::ambient::{self, Ambient};
use crate::shared::tracks::{LIBRARY_COLUMNS, album_columns};
use crate::shells::Shell;
use crate::shells::workspace::Workspace;
use crate::{
    Adaptive, ArtistView, DetailView, EqualizerView, FullscreenView, GenreView, HistoryView,
    HomeView, LibraryView, LoginView, SettingsView, SongView, UserView,
};

struct Screens {
    home: Entity<HomeView>,
    history: Entity<HistoryView>,
    library: Entity<LibraryView>,
    local: Entity<LibraryView>,
    artist: Option<Entity<ArtistView>>,
    artist_detail: Option<Entity<ArtistDetail>>,
    album: Option<Entity<DetailView>>,
    album_detail: Option<Entity<Detail>>,
    song: Entity<SongView>,
    song_detail: Entity<SongDetail>,
    user: Entity<UserView>,
    user_profile: Entity<Profile>,
    playlist: Option<Entity<DetailView>>,
    playlist_detail: Option<Entity<Detail>>,
    search: Entity<SearchView>,
    genres: Entity<Genres>,
    genre: Option<Entity<GenreView>>,
    genre_detail: Option<Entity<GenreDetails>>,
    /// The settings pages' view, whose search the settings header draws.
    settings: Entity<SettingsView>,
    /// The search field and page tabs floating over every settings page.
    settings_header: Entity<SettingsHeader>,
    equalizer: Entity<EqualizerView>,
    /// Whether a settings destination was on show after the last move, so the settings view
    /// hears when settings open and close.
    settings_open: bool,
}

struct Shells {
    workspace: Entity<Workspace>,
    fullscreen: Entity<FullscreenView>,
}

/// Which shell fills the window.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RootView {
    Workspace,
    Fullscreen,
}

impl RootView {
    /// The shell that draws `destination`.
    fn of(destination: &Destination) -> Self {
        match destination {
            Destination::Fullscreen => Self::Fullscreen,
            _ => Self::Workspace,
        }
    }
}

enum Focus {
    Search,
    Workspace,
    Fullscreen,
}

pub struct Root {
    session: Entity<Session>,
    playback: Entity<Playback>,
    io: Io,
    login: Entity<LoginView>,
    title_bar: Entity<TitleBar>,
    ambient: Entity<Ambient>,
    /// The question before a Jam link is joined, drawn over whichever shell is up so a link
    /// opened in fullscreen is answered there.
    jam_invite: Entity<JamInvite>,
    /// The question before a linked theme is installed, drawn over whichever shell is up.
    theme_invite: Entity<ThemeInvite>,
    shells: Shells,
    view: RootView,
    signing_in: bool,
    toolbar: Option<Entity<Toolbar>>,
    pending: Option<Focus>,
    navigation_transition: Option<Task<()>>,
    /// The entrance the shell plays as the window moves from one shell to another.
    shell_entrance: Option<Entrance>,
    /// The OS fullscreen move the next render owes the fullscreen view, true as it opens and
    /// false as it closes.
    os_follow: Option<bool>,
    /// Whether the window is in OS fullscreen because opening the fullscreen view put it there.
    os_entered: bool,
    screens: Screens,
    adaptive: Entity<Adaptive>,
    background: Option<gpui::WindowBackgroundAppearance>,
    /// The font the window is set in and the name it was built from, rebuilt only when the
    /// chosen font changes since the root renders every frame anything below it animates.
    font: Option<(String, Font)>,
    #[cfg(target_os = "windows")]
    rounded: Option<ui::Rounding>,
    /// The corners and traffic-light position last handed to AppKit.
    #[cfg(target_os = "macos")]
    framed: Option<(ui::Rounding, gpui::Point<gpui::Pixels>)>,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    decorations: gpui::WindowDecorations,
}

impl Root {
    pub fn new(
        session: Entity<Session>,
        library: Entity<Library>,
        playback: Entity<Playback>,
        queue: Entity<Queue>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&session, |this, session, cx| {
            if matches!(session.read(cx).state(), SessionState::SignedOut) {
                this.navigation_transition = None;
                this.shell_entrance = None;
                this.shells
                    .workspace
                    .update(cx, |workspace, cx| workspace.finish_transition(cx));
                this.screens.artist = None;
                this.screens.artist_detail = None;
                this.screens.album = None;
                this.screens.album_detail = None;
                this.screens.playlist = None;
                this.screens.playlist_detail = None;
            }
            cx.notify();
        })
        .detach();

        cx.subscribe(&session, |_, session, event, cx| {
            if matches!(event, SessionEvent::SignedIn) && !session.read(cx).authenticated() {
                let settings = Sonora::global(cx).settings.clone();
                let startup = settings.read(cx).startup().to_owned();
                if Screen::from_id(&startup).is_some_and(Screen::needs_account) {
                    settings.update(cx, |settings, cx| {
                        settings.set_startup(Screen::Home.id(), cx);
                    });
                }
                if matches!(
                    router::trail(cx).read(cx).current(),
                    Destination::Library(_)
                ) {
                    navigate(Destination::Home, cx);
                }
            }
        })
        .detach();

        let login = cx.new(|cx| LoginView::new(session.clone(), cx));

        let navigation = router::trail(cx);

        cx.subscribe(&navigation, |this, _, event, cx| {
            let NavigationEvent::Moved(destination) = event;
            this.transition_to(destination.clone(), cx);
        })
        .detach();

        cx.subscribe(&Network::global(cx), |this, _, _: &Reconnected, cx| {
            this.reload(cx)
        })
        .detach();

        let library_view = cx.new(|cx| {
            LibraryView::new(
                Shelf::Streaming,
                library.clone(),
                playback.clone(),
                window,
                cx,
            )
        });
        let local_view = cx.new(|cx| {
            LibraryView::new(Shelf::Local, library.clone(), playback.clone(), window, cx)
        });

        let io = Io::global(cx);
        let home_state = cx.new(|cx| Home::new(library.clone(), session.clone(), io.clone(), cx));
        let home = cx.new(|cx| HomeView::new(home_state, playback.clone(), cx));
        let history = Sonora::global(cx).history.clone();
        let history = cx.new(|cx| HistoryView::new(history, playback.clone(), window, cx));

        let queries = cx.new(|cx| Search::new(session.clone(), library.clone(), io.clone(), cx));
        let suggestions =
            cx.new(|cx| Search::new(session.clone(), library.clone(), io.clone(), cx));
        let genres = cx.new(|cx| Genres::new(session.clone(), io.clone(), cx));
        let search = cx.new(|cx| {
            SearchView::new(
                queries,
                suggestions,
                genres.clone(),
                playback.clone(),
                window,
                cx,
            )
        });

        let settings = cx.new(|cx| SettingsView::new(session.clone(), playback.clone(), cx));
        let equalizer = Sonora::global(cx).equalizer.clone();
        let equalizer = cx.new(|cx| EqualizerView::new(equalizer, window, cx));
        let settings_header =
            cx.new(|cx| SettingsHeader::new(settings.clone(), equalizer.clone(), cx));

        let song_detail = cx.new(|cx| SongDetail::new(session.clone(), io.clone(), cx));
        let song = cx.new(|cx| SongView::new(song_detail.clone(), playback.clone(), cx));

        let user_profile = cx.new(|cx| Profile::new(session.clone(), io.clone(), cx));
        let user = cx.new(|cx| UserView::new(user_profile.clone(), playback.clone(), cx));

        let start = navigation.read(cx).current();
        let workspace = cx.new(|cx| {
            Workspace::new(
                playback.clone(),
                queue.clone(),
                library_view.clone().into(),
                cx,
            )
        });
        let fullscreen = cx.new(|cx| FullscreenView::new(playback.clone(), queue.clone(), cx));
        let ambient = cx.new(Ambient::new);

        let title_bar = cx.new(TitleBar::new);
        cx.subscribe(&title_bar, |this, _, event, cx| match event {
            TitleBarEvent::ToggleSidebar => this
                .shells
                .workspace
                .update(cx, |workspace, cx| workspace.toggle_sidebar(cx)),
            TitleBarEvent::ToggleSidebarRight => this
                .shells
                .workspace
                .update(cx, |workspace, cx| workspace.toggle_sidebar_right(cx)),
        })
        .detach();

        // Re-read the system preference when the window becomes active instead of keeping a
        // long-lived portal listener alive. This picks up changes after the user returns from
        // the desktop accessibility settings.
        cx.observe_window_activation(window, |_, window, cx| {
            if !window.is_window_active() {
                return;
            }
            // Pick up plays made on other devices while Sonora was in the background, off the
            // window coming back to the foreground rather than a poll.
            Sonora::global(cx)
                .history
                .clone()
                .update(cx, |history, cx| history.refresh(cx));
            ui::refresh_primary_paste(cx);
            let settings = Sonora::global(cx).settings.clone();
            let (stillness, pace) = {
                let settings = settings.read(cx);
                (settings.stillness(), settings.pace())
            };
            if stillness != Stillness::System {
                return;
            }
            ui::motion::apply(stillness, pace, cx);
        })
        .detach();

        cx.observe_window_activation(window, |_, window, cx| update_focus_for_wake(window, cx))
            .detach();
        update_focus_for_wake(window, cx);

        window
            .observe_window_appearance(|_, cx| {
                let settings = Sonora::global(cx).settings.clone();
                let reported = ThemeKind::reported(cx);
                let changed = ThemeKind::assumed() != Some(reported);
                if changed {
                    ThemeKind::assume(reported);
                    settings.update(cx, |settings, cx| settings.set_system_theme(reported, cx));
                }
                if ThemeKind::from_id(settings.read(cx).theme()) != ThemeKind::System || !changed {
                    return;
                }
                let settings = settings.read(cx);
                let look = Look {
                    tint: cx.theme().tint,
                    ..settings.look()
                };
                let overrides = settings.theme_overrides();
                Theme::fade(look, &overrides, cx);
            })
            .detach();

        let adaptive = cx.new(|cx| Adaptive::new(playback.clone(), cx));

        let mut root = Self {
            session,
            playback,
            io,
            login,
            title_bar,
            shells: Shells {
                workspace: workspace.clone(),
                fullscreen,
            },
            view: RootView::Workspace,
            signing_in: false,
            toolbar: None,
            pending: None,
            navigation_transition: None,
            shell_entrance: None,
            os_follow: None,
            os_entered: false,
            screens: Screens {
                home,
                history,
                library: library_view,
                local: local_view,
                artist: None,
                artist_detail: None,
                album: None,
                album_detail: None,
                song,
                song_detail,
                user,
                user_profile,
                playlist: None,
                playlist_detail: None,
                search,
                genres,
                genre: None,
                genre_detail: None,
                settings,
                settings_header,
                equalizer,
                settings_open: false,
            },
            adaptive,
            ambient,
            jam_invite: cx.new(JamInvite::new),
            theme_invite: cx.new(ThemeInvite::new),
            background: None,
            font: None,
            #[cfg(target_os = "windows")]
            rounded: None,
            #[cfg(target_os = "macos")]
            framed: None,
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            decorations: Sonora::global(cx).settings.read(cx).window_decorations(),
        };
        root.show(start, cx);
        root
    }

    fn artist(&mut self, cx: &mut Context<Self>) -> (Entity<ArtistView>, Entity<ArtistDetail>) {
        if let (Some(view), Some(detail)) = (&self.screens.artist, &self.screens.artist_detail) {
            return (view.clone(), detail.clone());
        }

        let detail = cx.new(|cx| {
            ArtistDetail::new(
                self.session.clone(),
                Sonora::global(cx).library.clone(),
                self.io.clone(),
                cx,
            )
        });
        let view = cx.new(|cx| ArtistView::new(detail.clone(), self.playback.clone(), cx));
        self.screens.artist = Some(view.clone());
        self.screens.artist_detail = Some(detail.clone());
        (view, detail)
    }

    fn genre(&mut self, cx: &mut Context<Self>) -> (Entity<GenreView>, Entity<GenreDetails>) {
        if let (Some(view), Some(detail)) = (&self.screens.genre, &self.screens.genre_detail) {
            return (view.clone(), detail.clone());
        }

        let genres = self.screens.genres.clone();
        let detail =
            cx.new(|cx| GenreDetails::new(self.session.clone(), genres, self.io.clone(), cx));
        let view = cx.new(|cx| GenreView::new(detail.clone(), self.playback.clone(), cx));
        self.screens.genre = Some(view.clone());
        self.screens.genre_detail = Some(detail.clone());
        (view, detail)
    }

    fn album(&mut self, cx: &mut Context<Self>) -> (Entity<DetailView>, Entity<Detail>) {
        if let (Some(view), Some(detail)) = (&self.screens.album, &self.screens.album_detail) {
            return (view.clone(), detail.clone());
        }
        let playcounts = self.session.read(cx).capabilities().playcounts;
        let detail = cx.new(|cx| {
            Detail::new(
                self.session.clone(),
                Sonora::global(cx).library.clone(),
                self.io.clone(),
                cx,
            )
        });
        let view = cx.new(|cx| {
            DetailView::new(
                detail.clone(),
                self.playback.clone(),
                album_columns(playcounts),
                true,
                "album",
                cx,
            )
        });
        self.screens.album = Some(view.clone());
        self.screens.album_detail = Some(detail.clone());
        (view, detail)
    }

    fn playlist(&mut self, cx: &mut Context<Self>) -> (Entity<DetailView>, Entity<Detail>) {
        if let (Some(view), Some(detail)) = (&self.screens.playlist, &self.screens.playlist_detail)
        {
            return (view.clone(), detail.clone());
        }
        let detail = cx.new(|cx| {
            Detail::new(
                self.session.clone(),
                Sonora::global(cx).library.clone(),
                self.io.clone(),
                cx,
            )
        });
        let view = cx.new(|cx| {
            DetailView::new(
                detail.clone(),
                self.playback.clone(),
                LIBRARY_COLUMNS,
                true,
                "playlist",
                cx,
            )
        });
        self.screens.playlist = Some(view.clone());
        self.screens.playlist_detail = Some(detail.clone());
        (view, detail)
    }

    fn open_search(&mut self, cx: &mut Context<Self>) {
        navigate(Destination::Search, cx);
        self.pending = Some(Focus::Search);
        cx.notify();
    }

    /// Focuses the filter of the page on show. In settings that is the settings search.
    fn open_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = router::trail(cx).read(cx).current();
        if current.in_settings() {
            let search = self.screens.settings.read(cx).search();
            search.update(cx, |search, cx| search.focus(window, cx));
            return;
        }
        let Some(toolbar) = self.toolbar.clone() else {
            return;
        };
        toolbar.update(cx, |toolbar, cx| toolbar.focus(window, cx));
    }

    fn show_side(&self, tab: SideTab, cx: &mut Context<Self>) {
        self.shells
            .workspace
            .update(cx, |workspace, cx| workspace.show_side(tab, cx));
    }

    /// Tells the adaptive theme and the wake lock whether fullscreen is up. The ambient
    /// background is painted out of the cover's hues, so fullscreen samples the cover even with
    /// the adaptive theme off, and leaving drops the tint again.
    fn announce_fullscreen(&self, fullscreen: bool, cx: &mut Context<Self>) {
        self.adaptive
            .update(cx, |adaptive, cx| adaptive.set_fullscreen(fullscreen, cx));
        let wake = Sonora::global(cx).wake.clone();
        wake.update(cx, |wake, cx| wake.set_fullscreen(fullscreen, cx));
    }

    /// Carries the window into or out of OS fullscreen with the fullscreen view when the setting
    /// asks for it. Closing the view leaves OS fullscreen only if opening it went there, so a
    /// window put there with F11 stays.
    fn follow_os_fullscreen(&mut self, window: &mut Window, cx: &App) {
        let Some(opening) = self.os_follow.take() else {
            return;
        };
        match opening {
            true => {
                if Sonora::global(cx).settings.read(cx).os_fullscreen() && !window.is_fullscreen() {
                    window.toggle_fullscreen();
                    self.os_entered = true;
                }
            }
            false => {
                if std::mem::take(&mut self.os_entered) && window.is_fullscreen() {
                    window.toggle_fullscreen();
                }
            }
        }
    }

    fn toggle_fullscreen(&mut self, cx: &mut Context<Self>) {
        match self.view {
            RootView::Workspace => navigate(Destination::Fullscreen, cx),
            RootView::Fullscreen => back(cx),
        }
    }

    /// Escape that nothing on the page took. A focused text field lets go of the keyboard first,
    /// on any screen. Otherwise it leaves fullscreen, and on a settings page it closes an open
    /// menu or dialog, then clears the search. It never leaves settings.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typing = window
            .context_stack()
            .iter()
            .any(|context| context.contains(INPUT_CONTEXT));
        if typing {
            match self.view {
                RootView::Workspace => self
                    .shells
                    .workspace
                    .update(cx, |workspace, cx| workspace.focus(window, cx)),
                RootView::Fullscreen => self
                    .shells
                    .fullscreen
                    .update(cx, |fullscreen, cx| fullscreen.focus(window, cx)),
            }
            return;
        }
        if let RootView::Fullscreen = self.view {
            back(cx);
            return;
        }
        match router::trail(cx).read(cx).current() {
            Destination::Equalizer => {
                self.screens.equalizer.update(cx, |view, cx| {
                    if view.escape() {
                        cx.notify();
                    }
                });
            }
            Destination::Settings(_) => {
                self.screens.settings.update(cx, |view, cx| view.escape(cx));
            }
            Destination::Artist(_) => {
                if let Some(artist) = self.screens.artist.clone() {
                    artist.update(cx, |view, cx| view.escape(cx));
                }
            }
            Destination::Song(_) => {
                self.screens.song.update(cx, |view, cx| view.escape(cx));
            }
            _ => {}
        }
    }

    fn options(&self, cx: &Context<Self>) -> TitleBarOptions {
        let content = self.toolbar.clone().map(Into::into);

        match self.view {
            RootView::Workspace => self.shells.workspace.read(cx).title_bar(content, cx),
            RootView::Fullscreen => self.shells.fullscreen.read(cx).title_bar(content, cx),
        }
    }

    /// Opens settings on the page they last showed, inside the workspace.
    fn open_settings(&mut self, cx: &mut Context<Self>) {
        router::open_settings(cx);
    }

    /// Drops what the current page shows and fetches it again from its provider, so the page
    /// sits in its loading state until the new data lands. Ctrl/Cmd+R, the menu and the
    /// title-bar button all land here while a window is open, and pages with nothing to reload
    /// ignore it. A page still loading ignores it too, so a held key or repeated presses cannot
    /// keep restarting the load. The title-bar button turns until the page has its data.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let destination = router::trail(cx).read(cx).current();
        let Some(busy) = self.loading(&destination, cx) else {
            return;
        };
        if busy(cx) {
            return;
        }
        self.reload_page(destination, cx);
        self.title_bar
            .update(cx, |bar, cx| bar.refreshing(busy, cx));
    }

    /// How to tell whether `destination` is still loading, or `None` for a page with nothing
    /// to reload.
    fn loading(&self, destination: &Destination, cx: &App) -> Option<Busy> {
        let library = Sonora::global(cx).library.clone();
        let shelf = |shelf: Shelf| -> Busy {
            Box::new(move |cx| matches!(library.read(cx).state(shelf), LibraryState::Loading))
        };
        Some(match destination {
            Destination::Home => {
                let home = self.screens.home.clone();
                Box::new(move |cx| home.read(cx).is_loading(cx))
            }
            Destination::History => {
                let history = Sonora::global(cx).history.clone();
                Box::new(move |cx| matches!(history.read(cx).state(), HistoryState::Loading))
            }
            Destination::Library(_) => shelf(Shelf::Streaming),
            Destination::Local(_) => {
                let session = Sonora::global(cx).session.clone();
                let local = shelf(Shelf::Local);
                Box::new(move |cx| session.read(cx).scanning() || local(cx))
            }
            Destination::Album(_) => {
                let detail = self.screens.album_detail.clone()?;
                Box::new(move |cx| detail.read(cx).is_loading())
            }
            Destination::Playlist(_) => {
                let detail = self.screens.playlist_detail.clone()?;
                Box::new(move |cx| detail.read(cx).is_loading())
            }
            Destination::Artist(_) => {
                let detail = self.screens.artist_detail.clone()?;
                Box::new(move |cx| detail.read(cx).is_loading())
            }
            Destination::Genre(_) => {
                let detail = self.screens.genre_detail.clone()?;
                Box::new(move |cx| detail.read(cx).is_loading())
            }
            Destination::Song(_) => {
                let detail = self.screens.song_detail.clone();
                Box::new(move |cx| detail.read(cx).is_loading())
            }
            Destination::User(_) => {
                let profile = self.screens.user_profile.clone();
                Box::new(move |cx| profile.read(cx).is_loading())
            }
            Destination::Search
            | Destination::Settings(_)
            | Destination::Equalizer
            | Destination::Fullscreen => return None,
        })
    }

    /// Drops what `destination` shows and asks its provider for it again, past the caches.
    fn reload_page(&mut self, destination: Destination, cx: &mut Context<Self>) {
        let library = Sonora::global(cx).library.clone();
        match destination {
            Destination::Home => self.screens.home.update(cx, |home, cx| home.refresh(cx)),
            Destination::History => {
                let history = Sonora::global(cx).history.clone();
                history.update(cx, |history, cx| history.reload(cx));
            }
            Destination::Library(_) => {
                library.update(cx, |library, cx| library.reload(Shelf::Streaming, cx))
            }
            Destination::Local(_) => {
                library.update(cx, |library, cx| library.reload(Shelf::Local, cx))
            }
            Destination::Album(id) => {
                if let Some(detail) = self.screens.album_detail.clone() {
                    detail.update(cx, |detail, cx| detail.refresh(cx));
                }
                library.update(cx, |library, cx| library.refresh(Shelf::of(&id), cx));
            }
            Destination::Playlist(id) => {
                if let Some(detail) = self.screens.playlist_detail.clone() {
                    detail.update(cx, |detail, cx| detail.refresh(cx));
                }
                library.update(cx, |library, cx| library.refresh(Shelf::of(&id), cx));
            }
            Destination::Artist(_) => {
                if let Some(detail) = self.screens.artist_detail.clone() {
                    detail.update(cx, |detail, cx| detail.refresh(cx));
                }
            }
            Destination::Genre(_) => {
                if let Some(detail) = self.screens.genre_detail.clone() {
                    detail.update(cx, |detail, cx| detail.refresh(cx));
                }
            }
            Destination::Song(_) => self
                .screens
                .song_detail
                .update(cx, |detail, cx| detail.refresh(cx)),
            Destination::User(_) => self
                .screens
                .user_profile
                .update(cx, |profile, cx| profile.refresh(cx)),
            Destination::Search
            | Destination::Settings(_)
            | Destination::Equalizer
            | Destination::Fullscreen => {}
        }
    }

    /// Moves to `destination`. A move to another shell plays the entrance over the whole
    /// window, a move within the workspace plays it over the page, and reduced motion skips
    /// both.
    fn transition_to(&mut self, destination: Destination, cx: &mut Context<Self>) {
        self.navigation_transition = None;
        self.shell_entrance = None;

        let next = RootView::of(&destination);
        let moving = next != self.view;
        if moving || next == RootView::Fullscreen || cx.reduce_motion() {
            self.shells
                .workspace
                .update(cx, |workspace, cx| workspace.finish_transition(cx));
            self.show(destination, cx);
            if moving && !cx.reduce_motion() {
                self.reveal_shell(cx);
            }
            return;
        }

        self.show(destination, cx);
        let enter = self
            .shells
            .workspace
            .update(cx, |workspace, cx| workspace.reveal_content(cx));
        self.navigation_transition = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(enter).await;
            this.update(cx, |this, cx| {
                this.navigation_transition = None;
                this.shells
                    .workspace
                    .update(cx, |workspace, cx| workspace.finish_transition(cx));
            })
            .ok();
        }));
    }

    /// Plays the page entrance over the whole shell, for a move from one shell to another.
    fn reveal_shell(&mut self, cx: &mut Context<Self>) {
        let entrance = Entrance::start();
        self.shell_entrance = Some(entrance);
        self.navigation_transition = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(entrance.span()).await;
            this.update(cx, |this, cx| {
                this.navigation_transition = None;
                this.shell_entrance = None;
                cx.notify();
            })
            .ok();
        }));
    }

    /// The font the window is set in, built again only once a different font is chosen.
    fn font(&mut self, cx: &App) -> Font {
        let chosen = Sonora::global(cx).settings.read(cx).font();
        match &self.font {
            Some((name, font)) if name == chosen => font.clone(),
            _ => {
                let font = ui_font(chosen);
                self.font = Some((chosen.to_owned(), font.clone()));
                font
            }
        }
    }

    /// How much of the shell its entrance still hides, asking for the next frame while it runs.
    /// A see-through window fades the shell itself, so every frame of that fade is a full
    /// refresh, or the workspace's cached views would replay the opacity they were first
    /// painted with.
    fn shell_hidden(&mut self, window: &mut Window, cx: &Context<Self>) -> f32 {
        let Some(entrance) = self.shell_entrance else {
            return 0.;
        };
        if cx.reduce_motion() {
            self.shell_entrance = None;
            return 0.;
        }
        if entrance.running() {
            match cx.theme().transparent {
                true => window.on_next_frame(|window, _| window.refresh()),
                false => window.request_animation_frame(),
            }
        }
        entrance.hidden()
    }

    /// Loads the screen on show again, which is what a page that gave up while the network was
    /// gone needs once it is back. Focus stays where the user left it, since nothing moved.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let destination = router::trail(cx).read(cx).current();
        let pending = self.pending.take();
        self.show(destination, cx);
        self.pending = pending;
    }

    /// Tells the settings views about a move. The equalizer reads its meters only while it is
    /// the page on show and the previews run only while settings are open, so every move tells
    /// them, settings or not.
    fn follow_settings(&mut self, destination: &Destination, cx: &mut Context<Self>) {
        let equalizer = matches!(destination, Destination::Equalizer);
        self.screens
            .equalizer
            .update(cx, |view, cx| view.set_visible(equalizer, cx));
        let open = destination.in_settings();
        if open != self.screens.settings_open {
            self.screens.settings_open = open;
            self.screens
                .settings
                .update(cx, |view, _| view.set_open(open));
        }
        if let Destination::Settings(page) = destination {
            let page = *page;
            self.screens
                .settings
                .update(cx, |view, cx| view.select(page, cx));
        }
    }

    fn show(&mut self, destination: Destination, cx: &mut Context<Self>) {
        clear_listing(cx);
        let opening = matches!(destination, Destination::Fullscreen);
        if opening != matches!(self.view, RootView::Fullscreen) {
            self.os_follow = Some(opening);
            self.shells
                .fullscreen
                .update(cx, |fullscreen, cx| fullscreen.set_shown(opening, cx));
        }
        // Leaving settings is what clears the note about the last scan, so every move tells it.
        let settings = destination.in_settings();
        Scan::global(cx).update(cx, |scan, cx| scan.viewing_settings(settings, cx));
        let home = matches!(destination, Destination::Home);
        self.screens
            .home
            .update(cx, |view, cx| view.set_visible(home, cx));
        self.follow_settings(&destination, cx);
        if let Destination::Fullscreen = destination {
            self.view = RootView::Fullscreen;
            self.announce_fullscreen(true, cx);
            self.pending = Some(Focus::Fullscreen);
            cx.notify();
            return;
        }
        self.announce_fullscreen(false, cx);
        self.view = RootView::Workspace;
        self.pending = Some(match destination {
            Destination::Search => Focus::Search,
            _ => Focus::Workspace,
        });

        let mut toolbar = None;
        let mut header = None;
        let cached = destination != Destination::Equalizer;

        let content: AnyView = match destination {
            Destination::Fullscreen => return,
            Destination::Home => self.screens.home.clone().into(),
            Destination::History => {
                let history = self.screens.history.clone();
                history.update(cx, |history, cx| history.refresh(cx));
                toolbar = Some(history.read(cx).toolbar());
                history.into()
            }
            Destination::Local(tab) => {
                let local = self.screens.local.clone();
                local.update(cx, |local, cx| local.select(tab.into(), cx));
                toolbar = Some(local.read(cx).toolbar());
                local.into()
            }
            Destination::Library(tab) => {
                self.screens
                    .library
                    .update(cx, |library, cx| library.select(tab.into(), cx));
                let library = self.screens.library.clone();
                toolbar = Some(library.read(cx).toolbar());
                library.into()
            }
            Destination::Album(id) => {
                let (album, detail) = self.album(cx);
                detail.update(cx, |detail, cx| detail.open_album(&id, cx));
                toolbar = Some(album.read(cx).toolbar());
                album.into()
            }
            Destination::Song(id) => {
                self.screens
                    .song_detail
                    .update(cx, |detail, cx| detail.open(&id, cx));
                self.screens.song.clone().into()
            }
            Destination::Playlist(id) => {
                let (playlist, detail) = self.playlist(cx);
                detail.update(cx, |detail, cx| detail.open_playlist(&id, cx));
                toolbar = Some(playlist.read(cx).toolbar());
                playlist.into()
            }
            Destination::User(id) => {
                self.screens
                    .user_profile
                    .update(cx, |profile, cx| profile.open(&id, cx));
                self.screens.user.clone().into()
            }
            Destination::Artist(id) => {
                let (artist, detail) = self.artist(cx);
                detail.update(cx, |artist, cx| artist.open(&id, cx));
                toolbar = Some(artist.read(cx).toolbar());
                artist.into()
            }
            Destination::Genre(id) => {
                let (genre, detail) = self.genre(cx);
                detail.update(cx, |detail, cx| detail.open(&id, cx));
                toolbar = Some(genre.read(cx).toolbar());
                genre.into()
            }
            Destination::Search => self.screens.search.clone().into(),
            Destination::Equalizer => {
                header = Some(self.screens.settings_header.clone().into());
                self.screens.equalizer.clone().into()
            }
            Destination::Settings(_) => {
                header = Some(self.screens.settings_header.clone().into());
                self.screens.settings.clone().into()
            }
        };

        self.toolbar = toolbar;

        self.shells.workspace.update(cx, |workspace, cx| {
            workspace.set_content(content, header, cached, cx)
        });
        cx.notify();
    }
}

const UI_FONT: &str = "Inter";

const SCRIPTS: [&str; 18] = [
    "Source Han Sans",
    "Noto Sans CJK JP",
    "Noto Sans CJK SC",
    "Noto Sans CJK TC",
    "Noto Sans CJK KR",
    "Noto Sans Arabic",
    "Noto Sans Hebrew",
    "Noto Sans Thai",
    "Noto Sans Devanagari",
    "Hiragino Sans",
    "PingFang SC",
    "Apple SD Gothic Neo",
    "Yu Gothic UI",
    "Microsoft YaHei UI",
    "Malgun Gothic",
    "Noto Color Emoji",
    "Apple Color Emoji",
    "Segoe UI Emoji",
];

fn ui_font(chosen: &str) -> Font {
    match chosen == SYSTEM_FONT {
        true => Font {
            fallbacks: Some(scripts(false).clone()),
            ..font(UI_FONT)
        },
        false => Font {
            fallbacks: Some(scripts(true).clone()),
            ..font(SharedString::from(chosen.to_owned()))
        },
    }
}

fn scripts(custom: bool) -> &'static FontFallbacks {
    static BUNDLED: std::sync::OnceLock<FontFallbacks> = std::sync::OnceLock::new();
    static CHOSEN: std::sync::OnceLock<FontFallbacks> = std::sync::OnceLock::new();
    let named = || SCRIPTS.iter().map(|name| (*name).to_owned());
    match custom {
        true => CHOSEN.get_or_init(|| {
            FontFallbacks::from_fonts(std::iter::once(UI_FONT.to_owned()).chain(named()).collect())
        }),
        false => BUNDLED.get_or_init(|| FontFallbacks::from_fonts(named().collect())),
    }
}

/// Tells the wake lock whether the window has focus, which the display lock needs.
fn update_focus_for_wake(window: &Window, cx: &mut App) {
    let focused = window.is_window_active();
    let wake = Sonora::global(cx).wake.clone();
    wake.update(cx, |wake, cx| wake.set_focused(focused, cx));
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // An account that could not be reached is still an account, so nothing about a lost
        // network puts the sign-in page up: the workspace stays, on what the library kept and
        // on the local files.
        let show_sign_in = match self.session.read(cx).state() {
            SessionState::SignedOut | SessionState::Failed(_) => true,
            SessionState::Restoring | SessionState::SignedIn(_) | SessionState::Offline(_) => false,
            SessionState::Authorizing(_) => self.signing_in,
        };
        self.signing_in = show_sign_in;

        match self.pending.take() {
            Some(Focus::Search) => self
                .screens
                .search
                .update(cx, |search, cx| search.focus(window, cx)),
            Some(Focus::Workspace) => self
                .shells
                .workspace
                .update(cx, |workspace, cx| workspace.focus(window, cx)),
            Some(Focus::Fullscreen) => self
                .shells
                .fullscreen
                .update(cx, |fullscreen, cx| fullscreen.focus(window, cx)),
            None => {}
        }
        self.follow_os_fullscreen(window, cx);

        let options = match show_sign_in {
            true => TitleBarOptions {
                border: false,
                ..TitleBarOptions::default()
            },
            false => self.options(cx),
        };
        self.title_bar
            .update(cx, |bar, cx| bar.set_options(options, cx));

        let theme = *cx.theme();
        window.set_rem_size(theme.font_size);
        let appearance = ui::backdrop(theme.blur_window, theme.transparent);
        if self.background != Some(appearance) {
            self.background = Some(appearance);
            window.set_background_appearance(appearance);
        }

        // Windows can only round a window's corners while it's opaque (DWM never rounds a
        // per-pixel-alpha window), so this only takes effect once `backdrop` above lands on
        // `Opaque`; Linux/FreeBSD round their own chrome directly instead, below.
        #[cfg(target_os = "windows")]
        {
            let rounding = match window.is_fullscreen() {
                true => ui::Rounding::Square,
                false => Sonora::global(cx).settings.read(cx).window_rounding(),
            };
            if self.rounded != Some(rounding) {
                self.rounded = Some(rounding);
                state::apply_window_rounding(window, rounding, cx);
            }
        }

        // AppKit lays the window out again while its toolbar changes and calls back into it, so
        // the change waits until this frame has drawn. Fullscreen has no corners to pick, and
        // AppKit keeps a fullscreen window's toolbar as a bar above the content, so the toolbar
        // goes on the way in and the frame is picked again on the way out.
        #[cfg(target_os = "macos")]
        match window.is_fullscreen() {
            true => {
                if self.framed.take().is_some() {
                    window.defer(cx, |window, cx| {
                        state::apply_window_rounding(window, ui::Rounding::Square, cx);
                    });
                }
            }
            false => {
                let settings = Sonora::global(cx).settings.read(cx);
                let framed = (
                    settings.window_rounding(),
                    crate::chrome::traffic_lights(
                        settings,
                        ui::snapped(theme.metrics.title_bar, window),
                    ),
                );
                if self.framed != Some(framed) {
                    self.framed = Some(framed);
                    window.defer(cx, move |window, cx| {
                        state::apply_window_rounding(window, framed.0, cx);
                        window.set_traffic_light_position(framed.1);
                    });
                }
            }
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            let decorations = Sonora::global(cx).settings.read(cx).window_decorations();
            if self.decorations != decorations {
                self.decorations = decorations;
                window.request_decorations(decorations);
            }
        }

        // GPUI can't clip a subtree to a rounded parent (its content mask is a plain
        // rectangle), so on Linux/FreeBSD each edge of the chrome that actually touches a
        // corner rounds itself to match — see `chrome::window_radius`, and `TitleBar` /
        // `PlayerBar` for the top and bottom edges. Rounding the root too keeps its own
        // background quad correct and costs nothing.
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(Sonora::global(cx).settings.read(cx), cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<gpui::Pixels> = None;

        // The ambient field covers the window whole and carries the window's own opacity, so
        // the page colour under it would only stack a second alpha beneath that and leave a
        // see-through fullscreen reading nearly solid.
        let ambient = matches!(self.view, RootView::Fullscreen) && ambient::shown(cx);

        // The shell enters the way a page enters the workspace. An opaque window fades it
        // under a scrim of the page colour, which leaves the workspace's cached views alone,
        // and a see-through one fades the shell itself. The scrim reaches the bottom corners
        // of the window, so it rounds them. No shell takes the veil. The renderer drops a
        // backdrop inside a filtered layer, so fullscreen's frosted controls would show flat
        // until the veil lifted.
        let hidden = self.shell_hidden(window, cx);
        let dissolving = theme.transparent;
        let shell = match self.view {
            RootView::Workspace => self.shells.workspace.clone().into_any_element(),
            RootView::Fullscreen => self.shells.fullscreen.clone().into_any_element(),
        };
        let shell = div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .when(hidden > 0. && dissolving, |this| this.opacity(1. - hidden))
                    .child(shell),
            )
            .when(hidden > 0. && !dissolving, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .when_some(radius, |this, radius| this.rounded_b(radius))
                        .bg(theme.background)
                        .opacity(hidden),
                )
            })
            .child(self.jam_invite.clone())
            .child(self.theme_invite.clone());

        let root = div()
            .relative()
            .flex()
            .font(self.font(cx))
            .flex_col()
            .size_full()
            .when_some(radius, |this, radius| {
                this.rounded(radius).overflow_hidden()
            })
            .when(!ambient, |this| this.bg(theme.background))
            .text_color(theme.foreground)
            .capture_any_mouse_down(|_, window, cx| {
                if ui::cancel_middle_scroll(cx) {
                    window.refresh();
                    cx.stop_propagation();
                }
            })
            .capture_any_mouse_up(|event, window, cx| {
                if event.button == MouseButton::Middle
                    && ui::release_middle_scroll(event.position, cx)
                {
                    window.refresh();
                }
            })
            .on_mouse_up_out(MouseButton::Middle, |event, window, cx| {
                if ui::release_middle_scroll(event.position, cx) {
                    window.refresh();
                }
            })
            .capture_key_down(|event, window, cx| {
                if event.keystroke.key == "escape" && ui::cancel_middle_scroll(cx) {
                    window.refresh();
                    cx.stop_propagation();
                }
            })
            .on_mouse_move(|event, window, cx| {
                ui::update_middle_scroll(event.position, window, cx);
            })
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                |_, _, cx| back(cx),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                |_, _, cx| forward(cx),
            )
            .on_action(cx.listener(|_, _: &NavigateBack, _, cx| back(cx)))
            .on_action(cx.listener(|_, _: &NavigateForward, _, cx| forward(cx)))
            .on_action(cx.listener(|this, _: &OpenFilter, window, cx| this.open_filter(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSearch, _, cx| this.open_search(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| this.open_settings(cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &ToggleFullscreen, _, cx| this.toggle_fullscreen(cx)))
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .on_action(|_: &MinimizeWindow, window, _| window.minimize_window())
            .on_action(|_: &ZoomWindow, window, _| window.zoom_window())
            .on_action(|_: &ToggleWindowFullscreen, window, _| window.toggle_fullscreen())
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleQueue, _, cx| this.show_side(SideTab::Queue, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleLyrics, _, cx| this.show_side(SideTab::Lyrics, cx)),
            )
            // The ambient background sits behind everything, title bar included.
            .when(ambient, |this| this.child(self.ambient.clone()))
            // The bar only changes when it is told to, so a frame drawn for the lyrics or the
            // progress bar replays it rather than building it again.
            .child(
                self.title_bar.clone().cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(ui::snapped(theme.metrics.title_bar, window))
                        .flex_none(),
                ),
            )
            .when_else(
                show_sign_in,
                |this| this.child(div().flex().flex_1().min_h_0().child(self.login.clone())),
                |this| this.child(shell),
            );
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let root = root.child(WindowFrame::new());
        root
    }
}
