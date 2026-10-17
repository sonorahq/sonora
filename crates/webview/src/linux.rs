//! The Linux backend: a GTK window holding a WebKitGTK view over an ephemeral web context.
//!
//! Nothing links webkit2gtk. GPUI talks to X11 or Wayland itself and the app has no GTK anywhere
//! else, so the library is opened at runtime with `dlopen` and a system without it simply answers
//! `supported() == false`. The Flatpak builds on the GNOME runtime for the webkit2gtk-4.1 it
//! ships. `dlsym` walks a handle's dependencies, so the one webkit2gtk handle resolves gtk, glib,
//! gobject and soup too and no other soname is named here.
//!
//! GTK may be initialised once per process and only ever touched from the thread that did it, so
//! one resident thread owns every sign-in window. It parks on a condvar while no window is up,
//! runs `gtk_main` while one is, and services the poll side from a timer inside that loop. The two
//! threads share nothing but `Session`.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_ulong, c_void};
use std::ptr;
use std::sync::{Arc, Condvar, Mutex, Once, OnceLock};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};

use crate::native::{Fetch, HEIGHT, MIN_HEIGHT, MIN_WIDTH, Reading, WIDTH, host};
use crate::{Cookie, Target};

/// The sonames to try, newest first. 4.1 and 4.0 differ only in the libsoup they carry, and every
/// symbol used here is spelled the same in both.
const LIBRARIES: [&str; 2] = ["libwebkit2gtk-4.1.so.0\0", "libwebkit2gtk-4.0.so.37\0"];

/// How often the GTK thread reads the poll side's requests while a window is up.
const TICK: c_uint = 50;
/// How long a window waits for the GTK thread to reach a display before giving up on it.
const SETUP: Duration = Duration::from_secs(5);
/// Where the kernel lists every GPU with a link to the driver bound to it. One of the sysfs
/// subtrees a Flatpak sandbox shares, unlike `/sys/module` or `/proc/driver`.
const DRM: &str = "/sys/class/drm";
/// The first WebKitGTK `(major, minor)` whose DMA-BUF renderer runs the web process out of memory.
const RUNAWAY_DMABUF: (c_uint, c_uint) = (2, 54);

/// Firefox on Linux. WebKitGTK's own agent claims Safari on X11, a browser that does not exist,
/// and Google answers a browser it cannot place with "this browser may not be secure".
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:134.0) Gecko/20100101 Firefox/134.0";

const GTK_WINDOW_TOPLEVEL: c_int = 0;
const GTK_WIN_POS_CENTER: c_int = 1;
const G_SOURCE_REMOVE: Bool = 0;
const G_SOURCE_CONTINUE: Bool = 1;
/// `WEBKIT_COOKIE_POLICY_ACCEPT_ALWAYS`. WebKitGTK alone defaults to `ACCEPT_NO_THIRD_PARTY`;
/// the other two backends take every cookie, and a session that dies with the window has no third
/// party worth keeping out.
const WEBKIT_COOKIE_POLICY_ACCEPT_ALWAYS: c_int = 0;
/// `WEBKIT_USER_CONTENT_INJECT_TOP_FRAME`: the script runs in the page and in none of its frames.
const WEBKIT_USER_CONTENT_INJECT_TOP_FRAME: c_int = 0;
/// `WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START`: before the page's own scripts, which is what a
/// script that has to intercept one of them needs.
const WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START: c_int = 0;

type Ptr = *mut c_void;
type Bool = c_int;
/// `GCallback`: what a signal handler is cast to on the way into glib.
type Callback = unsafe extern "C" fn();
/// `GDestroyNotify`, and the shape of `soup_cookie_free`.
type Notify = unsafe extern "C" fn(Ptr);
/// `GSourceFunc`: the timer that services live windows.
type Source = unsafe extern "C" fn(Ptr) -> Bool;
/// `GAsyncReadyCallback`: how a cookie read reports back.
type Ready = unsafe extern "C" fn(Ptr, Ptr, Ptr);

/// A `GList` node, and a `GError`, as far as walking a cookie list and reporting a failure need
/// them. Both are laid out to match glib and never built here, so the fields nothing reads are
/// only there to put the ones that matter at the right offset.
#[repr(C)]
struct Node {
    data: Ptr,
    next: *mut Node,
    _prev: *mut Node,
}

#[repr(C)]
struct Fault {
    _domain: u32,
    _code: c_int,
    message: *const c_char,
}

macro_rules! symbols {
    ($($name:ident: unsafe extern "C" fn($($arg:ty),* $(,)?) $(-> $ret:ty)?,)*) => {
        /// Every call this backend makes, resolved once out of the webkit2gtk handle.
        struct Api {
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        impl Api {
            /// Resolves the whole table or nothing: a library missing one of these is too old to
            /// drive, and half a table would fail later and further from the cause.
            unsafe fn load(handle: Ptr) -> Option<Self> {
                Some(Self {
                    $($name: unsafe { symbol(handle, concat!(stringify!($name), "\0"))? },)*
                })
            }
        }
    };
}

symbols! {
    webkit_get_major_version: unsafe extern "C" fn() -> c_uint,
    webkit_get_minor_version: unsafe extern "C" fn() -> c_uint,
    gdk_set_allowed_backends: unsafe extern "C" fn(*const c_char),
    gdk_display_get_default: unsafe extern "C" fn() -> Ptr,
    g_type_name_from_instance: unsafe extern "C" fn(Ptr) -> *const c_char,
    gtk_disable_setlocale: unsafe extern "C" fn(),
    gtk_init_check: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char) -> Bool,
    gtk_main: unsafe extern "C" fn(),
    gtk_main_quit: unsafe extern "C" fn(),
    gtk_window_new: unsafe extern "C" fn(c_int) -> Ptr,
    gtk_window_set_title: unsafe extern "C" fn(Ptr, *const c_char),
    gtk_window_set_default_size: unsafe extern "C" fn(Ptr, c_int, c_int),
    gtk_window_set_position: unsafe extern "C" fn(Ptr, c_int),
    gtk_widget_set_size_request: unsafe extern "C" fn(Ptr, c_int, c_int),
    gtk_container_add: unsafe extern "C" fn(Ptr, Ptr),
    gtk_widget_show_all: unsafe extern "C" fn(Ptr),
    gtk_widget_destroy: unsafe extern "C" fn(Ptr),
    g_signal_connect_data: unsafe extern "C" fn(Ptr, *const c_char, Callback, Ptr, Option<Notify>, c_int) -> c_ulong,
    g_timeout_add: unsafe extern "C" fn(c_uint, Source, Ptr) -> c_uint,
    g_object_unref: unsafe extern "C" fn(Ptr),
    g_list_free_full: unsafe extern "C" fn(*mut Node, Notify),
    g_error_free: unsafe extern "C" fn(*mut Fault),
    webkit_web_context_new_ephemeral: unsafe extern "C" fn() -> Ptr,
    webkit_web_context_get_cookie_manager: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_web_view_new_with_context: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_web_view_get_settings: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_web_view_get_user_content_manager: unsafe extern "C" fn(Ptr) -> Ptr,
    webkit_user_content_manager_add_script: unsafe extern "C" fn(Ptr, Ptr),
    webkit_user_script_new: unsafe extern "C" fn(*const c_char, c_int, c_int, *const *const c_char, *const *const c_char) -> Ptr,
    webkit_user_script_unref: unsafe extern "C" fn(Ptr),
    webkit_web_view_get_uri: unsafe extern "C" fn(Ptr) -> *const c_char,
    webkit_web_view_load_uri: unsafe extern "C" fn(Ptr, *const c_char),
    webkit_web_view_terminate_web_process: unsafe extern "C" fn(Ptr),
    webkit_settings_set_user_agent: unsafe extern "C" fn(Ptr, *const c_char),
    webkit_cookie_manager_set_accept_policy: unsafe extern "C" fn(Ptr, c_int),
    webkit_cookie_manager_get_cookies: unsafe extern "C" fn(Ptr, *const c_char, Ptr, Ready, Ptr),
    webkit_cookie_manager_get_cookies_finish: unsafe extern "C" fn(Ptr, Ptr, *mut *mut Fault) -> *mut Node,
    soup_cookie_get_name: unsafe extern "C" fn(Ptr) -> *const c_char,
    soup_cookie_get_value: unsafe extern "C" fn(Ptr) -> *const c_char,
    soup_cookie_get_domain: unsafe extern "C" fn(Ptr) -> *const c_char,
    soup_cookie_free: unsafe extern "C" fn(Ptr),
}

/// Everything the poll side and the GTK thread say to each other about one window.
struct Shared {
    /// The url the view reported at the last tick.
    uri: Option<String>,
    /// The window is gone: the user closed it, or it never opened at all.
    closed: bool,
    fetch: Fetch,
    /// The poll side wants a cookie read started.
    wanted: bool,
    /// The poll side wants this url loaded.
    load: Option<CString>,
    /// The poll side wants the window closed.
    dismissed: bool,
}

/// One sign-in window, as both threads hold it.
struct Session {
    target: Target,
    state: Mutex<Shared>,
}

/// How far the GTK thread got. A display it cannot reach is not worth retrying, so the failure is
/// remembered and every later `open` reports it without queueing anything.
enum Start {
    Booting,
    Running,
    Failed(String),
}

struct Queue {
    start: Start,
    /// Windows the GTK thread has not built yet.
    waiting: Vec<Arc<Session>>,
}

/// The process-wide GTK thread, and the requests it has not picked up.
struct Host {
    queue: Mutex<Queue>,
    /// Signalled when a request arrives, to wake the thread out of its park, and when the thread
    /// has finished starting, to release whoever is waiting to queue the first one.
    work: Condvar,
}

/// One window as the GTK thread holds it. The pointers are only ever touched on that thread.
struct Live {
    session: Arc<Session>,
    window: Ptr,
    view: Ptr,
    context: Ptr,
    cookies: Ptr,
    /// What the cookie manager is asked for: the landing page, so that its own host and the parent
    /// domain both come back.
    uri: CString,
}

pub(crate) struct Window {
    session: Arc<Session>,
}

static HOST: OnceLock<Host> = OnceLock::new();
static THREAD: Once = Once::new();

thread_local! {
    /// The windows the GTK thread is driving. Reached only from the timer, on that thread.
    static LIVE: RefCell<Vec<Live>> = const { RefCell::new(Vec::new()) };
}

/// Whether webkit2gtk is installed. The first call starts this executable again with
/// [`crate::PROBE`] and waits for its answer, around 60ms, so the library and its GTK thread only
/// load into this process once a window actually opens. Loading them costs about 50 MiB.
pub(crate) fn supported() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| {
        let probed = std::process::Command::new("/proc/self/exe")
            .arg(crate::PROBE)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match probed {
            Ok(status) => status.success(),
            Err(error) => {
                log::warn!("webview: cannot look for webkit2gtk in a child: {error}");
                api().is_some()
            }
        }
    })
}

/// Answers a [`crate::PROBE`] launch with whether webkit2gtk loads, or none for any other launch.
pub(crate) fn probed() -> Option<bool> {
    std::env::args()
        .nth(1)
        .filter(|argument| argument == crate::PROBE)?;
    Some(api().is_some())
}

impl Window {
    /// Queues the window and returns. Only a GTK thread that cannot reach a display fails here;
    /// building the window itself takes half a second the first time, and waiting that out would
    /// freeze the click that asked for it. A window that then fails to build reports itself
    /// `closed`, which the caller already reads as a cancelled sign-in.
    pub(crate) fn open(target: &Target) -> Result<Self> {
        let api = api().context("webkit2gtk is not installed")?;
        let session = Arc::new(Session {
            target: target.clone(),
            state: Mutex::new(Shared {
                uri: None,
                closed: false,
                fetch: Fetch::Idle,
                wanted: false,
                load: None,
                dismissed: false,
            }),
        });
        gtk(api).submit(session.clone())?;
        Ok(Self { session })
    }

    /// True once the window is gone, whether the user closed it or it never came up.
    pub(crate) fn closed(&self) -> bool {
        self.session.state.lock().unwrap().closed
    }

    pub(crate) fn host(&self) -> Option<String> {
        let uri = self.session.state.lock().unwrap().uri.clone()?;
        host(&uri)
    }

    /// Hands back a finished cookie read, or asks the GTK thread for one when none is in flight.
    pub(crate) fn fetch(&mut self) -> Option<Vec<Cookie>> {
        let mut state = self.session.state.lock().unwrap();
        match state.fetch.take() {
            Reading::Done(cookies) => Some(cookies),
            Reading::Waiting => None,
            Reading::Start => {
                state.wanted = true;
                None
            }
        }
    }

    /// Asks the GTK thread to navigate the view to `url`. It starts within a tick.
    pub(crate) fn load(&self, url: &str) {
        let Ok(url) = CString::new(url) else {
            return;
        };
        self.session.state.lock().unwrap().load = Some(url);
    }

    /// Asks the GTK thread to take the window down. It is gone within a tick.
    pub(crate) fn close(&self) {
        self.session.state.lock().unwrap().dismissed = true;
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.close();
    }
}

impl Host {
    /// Queues a window for the GTK thread, waiting out its startup the first time and refusing
    /// outright if it never reached a display.
    fn submit(&self, session: Arc<Session>) -> Result<()> {
        let queue = self.queue.lock().unwrap();
        let (mut queue, _) = self
            .work
            .wait_timeout_while(queue, SETUP, |queue| matches!(queue.start, Start::Booting))
            .unwrap();
        match &queue.start {
            Start::Failed(why) => bail!("{why}"),
            Start::Booting => bail!("the sign-in window is still looking for a display"),
            Start::Running => {}
        }
        queue.waiting.push(session);
        self.work.notify_all();
        Ok(())
    }

    /// Blocks the GTK thread until there is a window to build.
    fn park(&self) {
        let mut queue = self.queue.lock().unwrap();
        while queue.waiting.is_empty() {
            queue = self.work.wait(queue).unwrap();
        }
    }

    /// Takes everything queued since the last tick.
    fn take(&self) -> Vec<Arc<Session>> {
        std::mem::take(&mut self.queue.lock().unwrap().waiting)
    }

    fn idle(&self) -> bool {
        self.queue.lock().unwrap().waiting.is_empty()
    }

    fn running(&self) {
        self.queue.lock().unwrap().start = Start::Running;
        self.work.notify_all();
    }

    /// Gives up on GTK for the life of the process, and on everything queued for it.
    fn fail(&self, why: String) {
        log::warn!("webview: {why}");
        let mut queue = self.queue.lock().unwrap();
        queue.start = Start::Failed(why);
        for session in std::mem::take(&mut queue.waiting) {
            session.state.lock().unwrap().closed = true;
        }
        self.work.notify_all();
    }
}

impl Live {
    /// Builds the window and starts loading the sign-in page. Runs on the GTK thread.
    fn open(api: &'static Api, session: Arc<Session>) -> Result<Self> {
        if session.state.lock().unwrap().dismissed {
            bail!("the sign-in was cancelled before the window opened");
        }
        let url =
            CString::new(session.target.url.as_str()).context("the sign-in url is not text")?;
        let title =
            CString::new(session.target.title.as_str()).context("the window title is not text")?;
        let uri = CString::new(format!("https://{}/", session.target.landing))
            .context("the landing host is not text")?;
        let agent = CString::new(session.target.agent.as_deref().unwrap_or(USER_AGENT))
            .context("the user agent is not text")?;

        // The context is the whole session: dropping it at the end takes the cookies with it.
        let context = unsafe { (api.webkit_web_context_new_ephemeral)() };
        if context.is_null() {
            bail!("cannot open a throwaway web session");
        }
        let view = unsafe { (api.webkit_web_view_new_with_context)(context) };
        let window = unsafe { (api.gtk_window_new)(GTK_WINDOW_TOPLEVEL) };
        if view.is_null() || window.is_null() {
            unsafe { (api.g_object_unref)(context) };
            bail!("cannot open the sign-in window");
        }
        let cookies = unsafe { (api.webkit_web_context_get_cookie_manager)(context) };
        // The session dies with the window, so there is no third party to keep out of it.
        unsafe {
            (api.webkit_cookie_manager_set_accept_policy)(
                cookies,
                WEBKIT_COOKIE_POLICY_ACCEPT_ALWAYS,
            )
        };
        let settings = unsafe { (api.webkit_web_view_get_settings)(view) };
        unsafe { (api.webkit_settings_set_user_agent)(settings, agent.as_ptr()) };

        unsafe { (api.gtk_window_set_title)(window, title.as_ptr()) };
        unsafe { (api.gtk_window_set_default_size)(window, WIDTH, HEIGHT) };
        unsafe { (api.gtk_window_set_position)(window, GTK_WIN_POS_CENTER) };
        unsafe { (api.gtk_widget_set_size_request)(window, MIN_WIDTH, MIN_HEIGHT) };
        // The view is floating until the window takes it, and dies with the window.
        unsafe { (api.gtk_container_add)(window, view) };

        // The handler holds one reference; `release` gives it back when the window goes.
        let held = Arc::into_raw(session.clone()) as Ptr;
        let handler = destroyed as unsafe extern "C" fn(Ptr, Ptr);
        unsafe {
            (api.g_signal_connect_data)(
                window,
                c"destroy".as_ptr(),
                std::mem::transmute::<unsafe extern "C" fn(Ptr, Ptr), Callback>(handler),
                held,
                Some(release),
                0,
            )
        };

        if let Some(source) = &session.target.script {
            inject(api, view, source);
        }
        // A scripted window is never looked at. GTK only needs it mapped for the user to see it,
        // and WebKit loads a page into an unmapped view all the same, so it stays hidden.
        if !session.target.scripted() {
            unsafe { (api.gtk_widget_show_all)(window) };
        }
        unsafe { (api.webkit_web_view_load_uri)(view, url.as_ptr()) };

        Ok(Self {
            session,
            window,
            view,
            context,
            cookies,
            uri,
        })
    }

    /// Carries one tick of a live window. False once the window is gone and reaped.
    fn service(&mut self, api: &'static Api) -> bool {
        let mut state = self.session.state.lock().unwrap();
        if state.closed {
            drop(state);
            unsafe { (api.g_object_unref)(self.context) };
            return false;
        }
        if state.dismissed {
            // `destroy` runs the handler that sets `closed`, which must not hold the lock.
            drop(state);
            // Destroying the widget and dropping the context is not enough: WebKit keeps the web
            // process alive afterwards, and with it the whole page it had loaded, which for the
            // minting page is most of a gigabyte. Killing it is the only thing that gives that
            // back, and the view is on its way out anyway.
            unsafe { (api.webkit_web_view_terminate_web_process)(self.view) };
            unsafe { (api.gtk_widget_destroy)(self.window) };
            return true;
        }
        state.uri = unsafe { text((api.webkit_web_view_get_uri)(self.view)) };
        let wanted = std::mem::take(&mut state.wanted);
        let load = state.load.take();
        // `collected` locks the same state, so the read is started with the lock let go.
        drop(state);
        if let Some(url) = load {
            unsafe { (api.webkit_web_view_load_uri)(self.view, url.as_ptr()) };
        }
        if wanted {
            let held = Arc::into_raw(self.session.clone()) as Ptr;
            unsafe {
                (api.webkit_cookie_manager_get_cookies)(
                    self.cookies,
                    self.uri.as_ptr(),
                    ptr::null_mut(),
                    collected,
                    held,
                )
            };
        }
        true
    }
}

/// The symbol table, loaded on the first call and kept for the life of the process. Unloading
/// webkit2gtk is not safe once glib has registered its types, so nothing ever calls `dlclose`.
fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(load).as_ref()
}

fn load() -> Option<Api> {
    for soname in LIBRARIES {
        let handle =
            unsafe { libc::dlopen(soname.as_ptr().cast(), libc::RTLD_LAZY | libc::RTLD_LOCAL) };
        if handle.is_null() {
            continue;
        }
        match unsafe { Api::load(handle) } {
            Some(api) => return Some(api),
            None => log::warn!(
                "webview: {} is missing symbols the sign-in window needs",
                soname.trim_end_matches('\0')
            ),
        }
    }
    None
}

/// Adds a user script to a view, to run in every page it loads once that page's own scripts have.
/// The script is copied into the content manager, so the one built here is released straight away.
fn inject(api: &'static Api, view: Ptr, source: &str) {
    let Ok(source) = CString::new(source) else {
        log::warn!("webview: the page script is not text");
        return;
    };
    let manager = unsafe { (api.webkit_web_view_get_user_content_manager)(view) };
    if manager.is_null() {
        log::warn!("webview: the view has no content manager, the page script will not run");
        return;
    }
    let script = unsafe {
        (api.webkit_user_script_new)(
            source.as_ptr(),
            WEBKIT_USER_CONTENT_INJECT_TOP_FRAME,
            WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START,
            ptr::null(),
            ptr::null(),
        )
    };
    if script.is_null() {
        log::warn!("webview: cannot build the page script");
        return;
    }
    unsafe { (api.webkit_user_content_manager_add_script)(manager, script) };
    unsafe { (api.webkit_user_script_unref)(script) };
}

/// Resolves one symbol. `dlsym` searches the handle's dependencies too, which is how gtk, glib and
/// soup are reached without naming a soname for any of them.
unsafe fn symbol<T>(handle: Ptr, name: &str) -> Option<T> {
    assert!(size_of::<T>() == size_of::<Ptr>());
    let found = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
    (!found.is_null()).then(|| unsafe { std::mem::transmute_copy(&found) })
}

/// The GTK thread, started on the first window and kept for the life of the process.
fn gtk(api: &'static Api) -> &'static Host {
    let host = HOST.get_or_init(|| Host {
        queue: Mutex::new(Queue {
            start: Start::Booting,
            waiting: Vec::new(),
        }),
        work: Condvar::new(),
    });
    THREAD.call_once(|| {
        let started = std::thread::Builder::new()
            .name("webview".to_string())
            .spawn(move || run(api, host));
        if let Err(error) = started {
            host.fail(format!("cannot start the sign-in thread: {error}"));
        }
    });
    host
}

/// Initialises GTK, then alternates between parking and running a main loop for as long as there
/// are windows. Never returns unless GTK itself refuses to start.
fn run(api: &'static Api, host: &'static Host) {
    // GDK opens its own connection to whichever server it finds, Wayland first and X11 through
    // XWayland otherwise, so the sign-in window never depends on an XWayland being present. A
    // GDK_BACKEND in the environment outranks this list, which is fine: either name works.
    unsafe { (api.gdk_set_allowed_backends)(c"wayland,x11".as_ptr()) };
    // SAFETY: nothing else in the process reads this variable, and no other thread is in GTK yet.
    if let Some(why) = shared_memory_only(api)
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
    {
        log::debug!("webview: {why}, disabling the dmabuf renderer");
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    // gtk_init would otherwise move the whole process onto the user's locale.
    unsafe { (api.gtk_disable_setlocale)() };
    if unsafe { (api.gtk_init_check)(ptr::null_mut(), ptr::null_mut()) } == 0 {
        return host.fail("cannot reach the display server".to_string());
    }
    // SAFETY: the display GTK just opened outlives the thread, and its type name is static.
    let backend = unsafe {
        let display = (api.gdk_display_get_default)();
        match display.is_null() {
            true => String::new(),
            false => CStr::from_ptr((api.g_type_name_from_instance)(display))
                .to_string_lossy()
                .into_owned(),
        }
    };
    log::debug!("webview: gdk opened {backend}");
    // On X11 WebKit tries to share a GL context with the main process and paints nothing; the
    // compositing env var has no public setting equivalent. Wayland needs the compositor on.
    // SAFETY: nothing else in the process reads this variable.
    if backend == "GdkX11Display" {
        unsafe { std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1") };
    }
    host.running();
    loop {
        host.park();
        // Built before the loop starts, so the first window does not wait out a tick.
        adopt(api, host);
        unsafe { (api.g_timeout_add)(TICK, tick, ptr::null_mut()) };
        unsafe { (api.gtk_main)() };
    }
}

/// Builds what has been queued, carries what is live, and ends the loop once nothing is left.
unsafe extern "C" fn tick(_data: Ptr) -> Bool {
    let (Some(api), Some(host)) = (api(), HOST.get()) else {
        return G_SOURCE_REMOVE;
    };
    adopt(api, host);
    LIVE.with_borrow_mut(|live| {
        live.retain_mut(|window| window.service(api));
        match live.is_empty() && host.idle() {
            true => {
                unsafe { (api.gtk_main_quit)() };
                G_SOURCE_REMOVE
            }
            false => G_SOURCE_CONTINUE,
        }
    })
}

/// Builds every window queued since the last look. Runs on the GTK thread, and on that thread
/// only, which is why it may touch `LIVE` at all.
fn adopt(api: &'static Api, host: &'static Host) {
    LIVE.with_borrow_mut(|live| {
        for session in host.take() {
            match Live::open(api, session.clone()) {
                Ok(window) => live.push(window),
                Err(error) => {
                    log::warn!("webview: cannot open the sign-in window: {error:#}");
                    session.state.lock().unwrap().closed = true;
                }
            }
        }
    });
}

/// The window is gone, either because the user closed it or because `service` took it down.
unsafe extern "C" fn destroyed(_window: Ptr, data: Ptr) {
    let session = unsafe { &*(data as *const Session) };
    session.state.lock().unwrap().closed = true;
}

/// A cookie read came back. Runs on the GTK thread, out of its main loop.
unsafe extern "C" fn collected(source: Ptr, result: Ptr, data: Ptr) {
    let session = unsafe { Arc::from_raw(data as *const Session) };
    let Some(api) = api() else {
        return;
    };
    let mut fault: *mut Fault = ptr::null_mut();
    let list =
        unsafe { (api.webkit_cookie_manager_get_cookies_finish)(source, result, &mut fault) };
    if !fault.is_null() {
        let why = unsafe { text((*fault).message) }.unwrap_or_default();
        unsafe { (api.g_error_free)(fault) };
        log::warn!("webview: cannot read the sign-in cookies: {why}");
        session.state.lock().unwrap().fetch = Fetch::Idle;
        return;
    }
    let cookies = unsafe { read(api, list) };
    unsafe { (api.g_list_free_full)(list, api.soup_cookie_free) };
    session.state.lock().unwrap().fetch = Fetch::Done(cookies);
}

/// Gives back the reference a glib callback was handed.
unsafe extern "C" fn release(data: Ptr) {
    drop(unsafe { Arc::from_raw(data as *const Session) });
}

/// Copies a `GList` of `SoupCookie` before the list is freed.
unsafe fn read(api: &'static Api, list: *mut Node) -> Vec<Cookie> {
    let mut cookies = Vec::new();
    let mut node = list;
    while !node.is_null() {
        let cookie = unsafe { (*node).data };
        let name = unsafe { text((api.soup_cookie_get_name)(cookie)) };
        let value = unsafe { text((api.soup_cookie_get_value)(cookie)) };
        let domain = unsafe { text((api.soup_cookie_get_domain)(cookie)) };
        if let (Some(name), Some(value), Some(domain)) = (name, value, domain) {
            cookies.push(Cookie {
                name,
                value,
                domain,
            });
        }
        node = unsafe { (*node).next };
    }
    cookies
}

/// Why WebKit's DMA-BUF renderer must stay off, or none when it may run. The NVIDIA driver
/// crashes it, and WebKitGTK 2.54 and newer grows the web process without bound on any GPU.
fn shared_memory_only(api: &'static Api) -> Option<&'static str> {
    if nvidia() {
        return Some("nvidia driver loaded");
    }
    let version = unsafe {
        (
            (api.webkit_get_major_version)(),
            (api.webkit_get_minor_version)(),
        )
    };
    (version >= RUNAWAY_DMABUF).then_some("webkitgtk 2.54 or newer")
}

/// Whether any GPU is bound to the NVIDIA driver, proprietary or open. Nouveau binds under its
/// own name and is not counted. A sysfs that cannot be read answers false, so a stripped
/// container keeps the accelerated path rather than losing the window.
fn nvidia() -> bool {
    let Ok(cards) = std::fs::read_dir(DRM) else {
        return false;
    };
    cards
        .flatten()
        .filter_map(|card| std::fs::read_link(card.path().join("device/driver")).ok())
        .any(|driver| driver.file_name().is_some_and(|name| name == "nvidia"))
}

/// Copies a borrowed C string. The caller keeps nothing of the original.
unsafe fn text(value: *const c_char) -> Option<String> {
    (!value.is_null()).then(|| {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    })
}
