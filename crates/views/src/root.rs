use gpui::{AnyView, Context, Entity, MouseButton, NavigationDirection, Render, Task};
use gpui::{App, Font, FontFallbacks, SharedString, font, prelude::*};
use gpui::{Window, div};
use input::{
    NavigateBack, NavigateForward, OpenFilter, OpenSearch, OpenSettings, SeekBack, SeekForward,
    ToggleFullscreen, ToggleLyrics, ToggleQueue,
};
use router::{Destination, NavigationEvent, SettingsTab, back, forward, navigate};
use state::{
    ArtistDetail, Detail, GenreDetails, Genres, Home, Io, Library, Playback, Profile, Queue,
    SYSTEM_FONT, Search, Session, SessionState, SideTab, SongDetail, Sonora,
};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use ui::WindowFrame;
use ui::{ActiveTheme as _, Dismiss, Look, Theme, ThemeKind, clear_listing};

use crate::chrome::{TitleBar, TitleBarEvent, TitleBarOptions, Toolbar, Tooled};
use crate::screens::search::SearchView;
use crate::shared::tracks::{LIBRARY_COLUMNS, album_columns};
use crate::shells::Shell;
use crate::shells::workspace::Workspace;
use crate::{
    Adaptive, ArtistView, DetailView, FullscreenView, GenreView, HistoryView, HomeView,
    LibraryView, LoginView, SettingsView, Shelf, SongView, UserView,
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
    settings: Entity<SettingsView>,
}

struct Shells {
    workspace: Entity<Workspace>,
    fullscreen: Entity<FullscreenView>,
}

enum RootView {
    Workspace,
    Fullscreen,
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
    shells: Shells,
    view: RootView,
    signing_in: bool,
    toolbar: Option<Entity<Toolbar>>,
    pending: Option<Focus>,
    navigation_transition: Option<Task<()>>,
    screens: Screens,
    _adaptive: Entity<Adaptive>,
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

        let login = cx.new(|cx| LoginView::new(session.clone(), cx));

        let navigation = router::trail(cx);

        cx.subscribe(&navigation, |this, _, event, cx| {
            let NavigationEvent::Moved(destination) = event;
            this.transition_to(destination.clone(), cx);
        })
        .detach();

        let library_view = cx.new(|cx| {
            LibraryView::new(Shelf::Saved, library.clone(), playback.clone(), window, cx)
        });
        let local_view = cx.new(|cx| {
            LibraryView::new(Shelf::Local, library.clone(), playback.clone(), window, cx)
        });

        let io = Io::global(cx);
        let home_state = cx.new(|cx| Home::new(library.clone(), session.clone(), io.clone(), cx));
        let home = cx.new(|cx| HomeView::new(home_state, playback.clone(), cx));
        let history = Sonora::global(cx).history.clone();
        let history = cx.new(|cx| HistoryView::new(history, playback.clone(), window, cx));

        let search_library = library.clone();

        let queries = cx.new(|cx| Search::new(session.clone(), search_library, io.clone(), cx));
        let genres = cx.new(|cx| Genres::new(session.clone(), io.clone(), cx));
        let search = cx.new(|cx| SearchView::new(queries, genres.clone(), playback.clone(), cx));

        let settings = cx.new(|cx| SettingsView::new(session.clone(), playback.clone(), cx));

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

        window
            .observe_window_appearance(|_, cx| {
                let settings = Sonora::global(cx).settings.clone();
                if ThemeKind::from_id(settings.read(cx).theme()) != ThemeKind::System {
                    return;
                }
                let reported = ThemeKind::reported(cx);
                if ThemeKind::assumed() == Some(reported) {
                    return;
                }
                ThemeKind::assume(reported);
                settings.update(cx, |settings, cx| settings.set_system_theme(reported, cx));
                let settings = settings.read(cx);
                let look = Look {
                    tint: cx.theme().tint,
                    ..settings.look()
                };
                let overrides = settings.theme_overrides().clone();
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
            },
            _adaptive: adaptive,
        };
        root.show(start, cx);
        root
    }

    fn artist(&mut self, cx: &mut Context<Self>) -> (Entity<ArtistView>, Entity<ArtistDetail>) {
        if let (Some(view), Some(detail)) = (&self.screens.artist, &self.screens.artist_detail) {
            return (view.clone(), detail.clone());
        }

        let detail = cx.new(|cx| ArtistDetail::new(self.session.clone(), self.io.clone(), cx));
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
        let playcounts = self.session.read(cx).playcounts();
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

    fn open_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn toggle_fullscreen(&mut self, cx: &mut Context<Self>) {
        match self.view {
            RootView::Workspace => navigate(Destination::Fullscreen, cx),
            RootView::Fullscreen => back(cx),
        }
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if matches!(self.view, RootView::Fullscreen) {
            back(cx);
        }
    }

    fn options(&self, cx: &Context<Self>) -> TitleBarOptions {
        let content = self.toolbar.clone().map(Into::into);

        match self.view {
            RootView::Workspace => self.shells.workspace.read(cx).title_bar(content, cx),
            RootView::Fullscreen => self.shells.fullscreen.read(cx).title_bar(content, cx),
        }
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        navigate(Destination::Settings(SettingsTab::General), cx);
        self.pending = Some(Focus::Workspace);
        cx.notify();
    }

    fn transition_to(&mut self, destination: Destination, cx: &mut Context<Self>) {
        self.navigation_transition = None;

        let changes_shell = matches!(destination, Destination::Fullscreen)
            || matches!(self.view, RootView::Fullscreen);
        if changes_shell || cx.reduce_motion() {
            self.shells
                .workspace
                .update(cx, |workspace, cx| workspace.finish_transition(cx));
            self.show(destination, cx);
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

    fn show(&mut self, destination: Destination, cx: &mut Context<Self>) {
        clear_listing(cx);
        if let Destination::Fullscreen = destination {
            self.view = RootView::Fullscreen;
            self.pending = Some(Focus::Fullscreen);
            cx.notify();
            return;
        }
        self.view = RootView::Workspace;
        self.pending = Some(match destination {
            Destination::Search => Focus::Search,
            _ => Focus::Workspace,
        });

        let mut toolbar = None;

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
            Destination::Settings(tab) => {
                self.screens
                    .settings
                    .update(cx, |settings, cx| settings.select(tab, cx));
                self.screens.settings.clone().into()
            }
        };

        self.toolbar = toolbar;

        self.shells
            .workspace
            .update(cx, |workspace, cx| workspace.set_content(content, cx));
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

fn ui_font(cx: &App) -> Font {
    let chosen = Sonora::global(cx).settings.read(cx).font();
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

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let show_sign_in = match self.session.read(cx).state() {
            SessionState::SignedOut | SessionState::Failed(_) => true,
            SessionState::Restoring | SessionState::SignedIn(_) => false,
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

        let root = div()
            .relative()
            .flex()
            .font(ui_font(cx))
            .flex_col()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
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
            .on_action(cx.listener(|this, _: &SeekBack, _, cx| {
                this.playback
                    .update(cx, |playback, cx| playback.seek_back(cx));
            }))
            .on_action(cx.listener(|this, _: &SeekForward, _, cx| {
                this.playback
                    .update(cx, |playback, cx| playback.seek_forward(cx));
            }))
            .on_action(cx.listener(|this, _: &OpenFilter, window, cx| this.open_filter(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSearch, _, cx| this.open_search(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| this.open_settings(cx)))
            .on_action(cx.listener(|this, _: &ToggleFullscreen, _, cx| this.toggle_fullscreen(cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.dismiss(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleQueue, _, cx| this.show_side(SideTab::Queue, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleLyrics, _, cx| this.show_side(SideTab::Lyrics, cx)),
            )
            .child(self.title_bar.clone())
            .when_else(
                show_sign_in,
                |this| this.child(div().flex().flex_1().min_h_0().child(self.login.clone())),
                |this| {
                    this.child(match self.view {
                        RootView::Workspace => self.shells.workspace.clone().into_any_element(),
                        RootView::Fullscreen => self.shells.fullscreen.clone().into_any_element(),
                    })
                },
            );
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let root = root.child(WindowFrame::new());
        root
    }
}
