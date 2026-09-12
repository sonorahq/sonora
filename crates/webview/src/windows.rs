//! The Windows backend: a Win32 window hosting an InPrivate WebView2 controller. WebView2 and the
//! window both live on GPUI's main STA thread; its asynchronous setup and cookie callbacks are
//! dispatched by that thread's existing message loop.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use anyhow::{Context as _, Result};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2_2,
    ICoreWebView2Controller, ICoreWebView2Environment10,
};
use webview2_com::{
    CoTaskMemPWSTR, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, GetCookiesCompletedHandler,
};
use windows::Win32::Foundation::{E_POINTER, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetSystemMetrics, IsWindow, MINMAXINFO, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
    ShowWindow, WINDOW_EX_STYLE, WM_CLOSE, WM_GETMINMAXINFO, WM_NCDESTROY, WM_SIZE, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{Interface as _, PCWSTR, PWSTR, w};

use crate::native::{Fetch, HEIGHT, MIN_HEIGHT, MIN_WIDTH, Reading, WIDTH};
use crate::{Cookie, Target};

pub(crate) fn supported() -> bool {
    true
}

thread_local! {
    /// Controllers are keyed by their host window so the Win32 callback can resize them.
    static CONTROLLERS: RefCell<HashMap<isize, ICoreWebView2Controller>> = RefCell::new(HashMap::new());
}

pub(crate) struct Window {
    hwnd: HWND,
    browser: Rc<RefCell<Option<Browser>>>,
    fetch: Rc<RefCell<Fetch>>,
}

/// COM objects that must stay alive for as long as the sign-in window is usable.
struct Browser {
    controller: ICoreWebView2Controller,
    view: ICoreWebView2,
    cookies: webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2CookieManager,
}

impl Window {
    pub(crate) fn open(target: &Target) -> Result<Self> {
        let hwnd = create_window(&target.title)?;
        let browser = Rc::new(RefCell::new(None));
        if let Err(error) = begin_environment(hwnd, &target.url, Rc::downgrade(&browser)) {
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(error);
        }
        let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };

        Ok(Self {
            hwnd,
            browser,
            fetch: Rc::new(RefCell::new(Fetch::Idle)),
        })
    }

    /// True once the user closed and destroyed the host window.
    pub(crate) fn closed(&self) -> bool {
        !unsafe { IsWindow(Some(self.hwnd)) }.as_bool()
    }

    pub(crate) fn host(&self) -> Option<String> {
        let view = self.browser.borrow().as_ref()?.view.clone();
        let mut source = PWSTR::null();
        unsafe { view.Source(&mut source) }.ok()?;
        let source = CoTaskMemPWSTR::from(source).to_string();
        crate::native::host(&source)
    }

    /// Hands back a finished cookie fetch, or starts one when none is in flight.
    pub(crate) fn fetch(&mut self) -> Option<Vec<Cookie>> {
        let reading = self.fetch.borrow_mut().take();
        match reading {
            Reading::Done(cookies) => return Some(cookies),
            Reading::Waiting => return None,
            Reading::Start => {}
        }
        // The controller may not have arrived yet; the claimed slot goes back if it has not.
        let Some(cookies) = self
            .browser
            .borrow()
            .as_ref()
            .map(|browser| browser.cookies.clone())
        else {
            *self.fetch.borrow_mut() = Fetch::Idle;
            return None;
        };
        let slot = self.fetch.clone();
        let handler = GetCookiesCompletedHandler::create(Box::new(move |result, found| {
            let cookies = match result
                .and_then(|()| found.ok_or_else(|| windows::core::Error::from(E_POINTER)))
            {
                Ok(found) => read_cookies(&found),
                Err(error) => {
                    log::warn!("webview: cannot read sign-in cookies: {error}");
                    *slot.borrow_mut() = Fetch::Idle;
                    return Ok(());
                }
            };
            match cookies {
                Ok(cookies) => *slot.borrow_mut() = Fetch::Done(cookies),
                Err(error) => {
                    log::warn!("webview: cannot decode sign-in cookies: {error}");
                    *slot.borrow_mut() = Fetch::Idle;
                }
            }
            Ok(())
        }));
        if let Err(error) = unsafe { cookies.GetCookies(w!(""), &handler) } {
            log::warn!("webview: cannot start sign-in cookie fetch: {error}");
            *self.fetch.borrow_mut() = Fetch::Idle;
        }
        None
    }

    /// Navigates the view to `url`. Nothing happens before the controller has arrived.
    pub(crate) fn load(&self, url: &str) {
        let Some(view) = self
            .browser
            .borrow()
            .as_ref()
            .map(|browser| browser.view.clone())
        else {
            return;
        };
        let url = wide(url);
        if let Err(error) = unsafe { view.Navigate(PCWSTR(url.as_ptr())) } {
            log::warn!("webview: cannot load the sign-in url again: {error}");
        }
    }

    pub(crate) fn close(&self) {
        if let Some(browser) = self.browser.borrow_mut().take() {
            let _ = unsafe { browser.controller.Close() };
        }
        if !self.closed() {
            let _ = unsafe { DestroyWindow(self.hwnd) };
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.close();
    }
}

/// Starts WebView2 without pumping a nested message loop inside GPUI's click handler.
fn begin_environment(hwnd: HWND, url: &str, browser: Weak<RefCell<Option<Browser>>>) -> Result<()> {
    let url = url.to_string();
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result, environment| {
            let result = result
                .context("WebView2 could not create an environment")
                .and_then(|()| environment.context("WebView2 returned no environment"))
                .and_then(|environment| begin_controller(environment, hwnd, url, browser));
            if let Err(error) = result {
                fail(hwnd, error);
            }
            Ok(())
        },
    ));
    let folder = user_data_folder()?;
    unsafe {
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(folder.as_ptr()),
            None,
            &handler,
        )
    }
    .context("cannot start the WebView2 environment")
}

/// Sonora's own cache folder for WebView2's browser state, as a wide string. WebView2 otherwise
/// writes beside the executable, which an installed copy under Program Files cannot do.
fn user_data_folder() -> Result<Vec<u16>> {
    let folder = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("sonora")
        .join("webview2");
    std::fs::create_dir_all(&folder)
        .with_context(|| format!("cannot create {}", folder.display()))?;
    Ok(wide(&folder.to_string_lossy()))
}

/// Configures an InPrivate profile and starts its controller asynchronously.
fn begin_controller(
    environment: webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment,
    hwnd: HWND,
    url: String,
    browser: Weak<RefCell<Option<Browser>>>,
) -> Result<()> {
    let environment = environment
        .cast::<ICoreWebView2Environment10>()
        .context("the installed WebView2 runtime does not support private profiles")?;
    let options = unsafe { environment.CreateCoreWebView2ControllerOptions() }
        .context("cannot configure the private sign-in profile")?;
    unsafe { options.SetProfileName(w!("Sonora Sign-In")) }
        .context("cannot name the sign-in profile")?;
    unsafe { options.SetIsInPrivateModeEnabled(true) }
        .context("cannot make the sign-in profile private")?;

    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            let result = result
                .context("WebView2 could not create a controller")
                .and_then(|()| controller.context("WebView2 returned no controller"))
                .and_then(|controller| finish_controller(controller, hwnd, &url, &browser));
            if let Err(error) = result {
                fail(hwnd, error);
            }
            Ok(())
        },
    ));
    unsafe { environment.CreateCoreWebView2ControllerWithOptions(hwnd, &options, &handler) }
        .context("cannot start the private WebView2 controller")
}

/// Attaches a completed controller to the live window and starts navigation.
fn finish_controller(
    controller: ICoreWebView2Controller,
    hwnd: HWND,
    url: &str,
    browser: &Weak<RefCell<Option<Browser>>>,
) -> Result<()> {
    let Some(browser) = browser.upgrade() else {
        let _ = unsafe { controller.Close() };
        return Ok(());
    };
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        let _ = unsafe { controller.Close() };
        return Ok(());
    }

    let view = unsafe { controller.CoreWebView2() }.context("cannot open the sign-in webview")?;
    let settings = unsafe { view.Settings() }.context("cannot read webview settings")?;
    let settings = settings
        .cast::<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings2>()
        .context("the installed WebView2 runtime cannot set a user agent")?;
    let user_agent = browser_user_agent(&settings)?;
    unsafe { settings.SetUserAgent(PCWSTR(user_agent.as_ptr())) }
        .context("cannot set the sign-in user agent")?;
    let view2 = view
        .cast::<ICoreWebView2_2>()
        .context("the installed WebView2 runtime cannot read cookies")?;
    let cookies = unsafe { view2.CookieManager() }.context("cannot open the cookie store")?;

    resize(controller.clone(), hwnd);
    unsafe { controller.SetIsVisible(true) }.context("cannot show the sign-in webview")?;
    let url = wide(url);
    unsafe { view.Navigate(PCWSTR(url.as_ptr())) }.context("cannot load the sign-in url")?;
    CONTROLLERS.with(|controllers| {
        controllers
            .borrow_mut()
            .insert(key(hwnd), controller.clone());
    });
    *browser.borrow_mut() = Some(Browser {
        controller,
        view,
        cookies,
    });
    Ok(())
}

/// Closes an unusable host after an asynchronous setup failure.
fn fail(hwnd: HWND, error: anyhow::Error) {
    log::error!("webview: cannot prepare the sign-in window: {error:#}");
    let _ = unsafe { DestroyWindow(hwnd) };
}

/// Registers and creates the standalone Win32 host window with a centered client area.
fn create_window(title: &str) -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(None) }.context("cannot find the app module")?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: HINSTANCE(instance.0),
        lpszClassName: w!("SonoraSignInWebView"),
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };

    let mut bounds = RECT {
        left: 0,
        top: 0,
        right: WIDTH,
        bottom: HEIGHT,
    };
    unsafe { AdjustWindowRectEx(&mut bounds, WS_OVERLAPPEDWINDOW, false, WINDOW_EX_STYLE(0)) }
        .context("cannot size the sign-in window")?;
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    let x = (unsafe { GetSystemMetrics(SM_CXSCREEN) } - width) / 2;
    let y = (unsafe { GetSystemMetrics(SM_CYSCREEN) } - height) / 2;
    let title = wide(title);
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SonoraSignInWebView"),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
    }
    .context("cannot create the sign-in window")?;
    Ok(hwnd)
}

/// Copies the WebView2 cookie list before its completion callback returns.
fn read_cookies(
    found: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2CookieList,
) -> Result<Vec<Cookie>> {
    let mut count = 0;
    unsafe { found.Count(&mut count) }.context("cannot count cookies")?;
    (0..count)
        .map(|index| {
            let cookie = unsafe { found.GetValueAtIndex(index) }.context("cannot read a cookie")?;
            Ok(Cookie {
                name: cookie_string(|value| unsafe { cookie.Name(value) })?,
                value: cookie_string(|value| unsafe { cookie.Value(value) })?,
                domain: cookie_string(|value| unsafe { cookie.Domain(value) })?,
            })
        })
        .collect()
}

/// Takes one COM-allocated string property from a WebView2 cookie.
fn cookie_string(read: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>) -> Result<String> {
    let mut value = PWSTR::null();
    read(&mut value).context("cannot read a cookie property")?;
    Ok(CoTaskMemPWSTR::from(value).to_string())
}

/// Keeps the runtime's current Chromium version while removing the Edge token Google uses to
/// reject an otherwise capable embedded browser.
fn browser_user_agent(
    settings: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings2,
) -> Result<Vec<u16>> {
    let mut value = PWSTR::null();
    unsafe { settings.UserAgent(&mut value) }.context("cannot read the WebView2 user agent")?;
    let value = CoTaskMemPWSTR::from(value).to_string();
    let value = value
        .split_whitespace()
        .filter(|part| !part.starts_with("Edg/"))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(wide(&value))
}

fn resize(controller: ICoreWebView2Controller, hwnd: HWND) {
    let mut bounds = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut bounds) }.is_ok() {
        let _ = unsafe { controller.SetBounds(bounds) };
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn key(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_SIZE => {
            CONTROLLERS.with(|controllers| {
                if let Some(controller) = controllers.borrow().get(&key(hwnd)) {
                    resize(controller.clone(), hwnd);
                }
            });
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let limits = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
            limits.ptMinTrackSize.x = MIN_WIDTH;
            limits.ptMinTrackSize.y = MIN_HEIGHT;
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_NCDESTROY => {
            CONTROLLERS.with(|controllers| {
                controllers.borrow_mut().remove(&key(hwnd));
            });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
