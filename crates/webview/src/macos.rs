//! The macOS backend: an `NSWindow` whose content view is a `WKWebView` over a non-persistent data
//! store. AppKit and WebKit are main-thread only, and so is every call here; WebKit runs the cookie
//! completion on the main thread too, which is what lets `Fetch` sit in an `Rc`.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;

use anyhow::{Context as _, Result};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly as _};
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{
    NSArray, NSHTTPCookie, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest,
};
use objc2_web_kit::{
    WKHTTPCookieStore, WKUserScript, WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
    WKWebsiteDataStore,
};

use crate::native::{Fetch, HEIGHT, MIN_HEIGHT, MIN_WIDTH, Reading, WIDTH};
use crate::{Cookie, Target};

/// Safari's own user agent. WebKit's default leaves out the `Version/… Safari/…` tail, and
/// Google refuses to sign in a browser it reads as embedded.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15";

pub(crate) fn supported() -> bool {
    true
}

pub(crate) struct Window {
    window: Retained<NSWindow>,
    view: Retained<WKWebView>,
    cookies: Retained<WKHTTPCookieStore>,
    fetch: Rc<RefCell<Fetch>>,
    /// Whether the window was put on the screen. A scripted window never is, and an invisible
    /// window would otherwise read as one the user had closed.
    shown: bool,
    /// Whether `close` has been called. This is what `closed` answers for a hidden window.
    dismissed: Rc<RefCell<bool>>,
}

impl Window {
    pub(crate) fn open(target: &Target) -> Result<Self> {
        let mtm =
            MainThreadMarker::new().context("the sign-in window has to open on the main thread")?;
        let url = NSURL::URLWithString(&NSString::from_str(&target.url))
            .context("cannot parse the sign-in url")?;
        let frame = NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(f64::from(WIDTH), f64::from(HEIGHT)),
        );

        // The view copies the configuration, so the store is read back off the view afterwards.
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
        unsafe { configuration.setWebsiteDataStore(&store) };
        let view = unsafe {
            WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &configuration)
        };
        unsafe { view.setCustomUserAgent(Some(&NSString::from_str(USER_AGENT))) };
        if let Some(source) = &target.script {
            // The view copied the configuration, so the script goes into the copy it kept.
            let script = unsafe {
                WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                    WKUserScript::alloc(mtm),
                    &NSString::from_str(source),
                    WKUserScriptInjectionTime::AtDocumentStart,
                    true,
                )
            };
            unsafe {
                view.configuration()
                    .userContentController()
                    .addUserScript(&script)
            };
        }
        let cookies = unsafe { view.configuration().websiteDataStore().httpCookieStore() };

        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // Closing must not free the window under the `Retained` this struct holds.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&target.title));
        window.setMinSize(NSSize::new(f64::from(MIN_WIDTH), f64::from(MIN_HEIGHT)));
        window.setContentView(Some(&view));
        window.center();
        // A scripted window is never looked at. WebKit drives a view in a window that was never
        // ordered front all the same, so it stays off the screen.
        let shown = !target.scripted();
        if shown {
            window.makeKeyAndOrderFront(None);
        }
        unsafe { view.loadRequest(&NSURLRequest::requestWithURL(&url)) };

        Ok(Self {
            window,
            view,
            cookies,
            fetch: Rc::new(RefCell::new(Fetch::Idle)),
            shown,
            dismissed: Rc::new(RefCell::new(false)),
        })
    }

    /// True once the window is gone. A window the user can see is gone when it is neither visible
    /// nor miniaturized; one that was never shown is gone only when the app closes it.
    pub(crate) fn closed(&self) -> bool {
        match self.shown {
            true => !self.window.isVisible() && !self.window.isMiniaturized(),
            false => *self.dismissed.borrow(),
        }
    }

    pub(crate) fn host(&self) -> Option<String> {
        let url = unsafe { self.view.URL() }?;
        url.host().map(|host| host.to_string())
    }

    /// Hands back a finished cookie fetch, or starts one when none is in flight.
    pub(crate) fn fetch(&mut self) -> Option<Vec<Cookie>> {
        let reading = self.fetch.borrow_mut().take();
        match reading {
            Reading::Done(cookies) => return Some(cookies),
            Reading::Waiting => return None,
            Reading::Start => {}
        }
        let slot = self.fetch.clone();
        let done = RcBlock::new(move |found: NonNull<NSArray<NSHTTPCookie>>| {
            let found = unsafe { found.as_ref() };
            let cookies = found
                .iter()
                .map(|cookie| Cookie {
                    name: cookie.name().to_string(),
                    value: cookie.value().to_string(),
                    domain: cookie.domain().to_string(),
                })
                .collect();
            *slot.borrow_mut() = Fetch::Done(cookies);
        });
        unsafe { self.cookies.getAllCookies(&done) };
        None
    }

    /// Navigates the view to `url`; a url that does not parse is ignored.
    pub(crate) fn load(&self, url: &str) {
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return;
        };
        unsafe { self.view.loadRequest(&NSURLRequest::requestWithURL(&url)) };
    }

    /// Evaluates `script` in the page world, where the user script runs too.
    pub(crate) fn run(&self, script: &str) {
        unsafe {
            self.view
                .evaluateJavaScript_completionHandler(&NSString::from_str(script), None)
        };
    }

    pub(crate) fn close(&self) {
        if !self.closed() {
            self.window.close();
        }
        *self.dismissed.borrow_mut() = true;
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.close();
    }
}
