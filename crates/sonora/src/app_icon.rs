use std::ffi::c_void;

use anyhow::{Context as _, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, HICON, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_DPICHANGED, WM_NCDESTROY, WM_SETICON,
    WM_SETTINGCHANGE,
};
use windows::core::PCWSTR;

const SUBCLASS: usize = 0x736f_6963;
/// GPUI and winresource use the first icon group in the executable.
const RESOURCE: PCWSTR = PCWSTR(1 as _);

/// Owns the small and large icons for one window until replacement or window destruction.
#[derive(Default)]
struct Icons {
    small: Option<HICON>,
    large: Option<HICON>,
}

impl Icons {
    fn refresh(&mut self, hwnd: HWND, dpi: u32) {
        if let Err(error) = self.replace(hwnd, dpi) {
            log::warn!("icon: cannot set the window icons: {error:#}");
        }
    }

    /// Loads owned handles because shared icon caching ignores the requested size.
    /// Failed loads leave the current pair in place.
    fn replace(&mut self, hwnd: HWND, dpi: u32) -> Result<()> {
        let mut next = Self::default();
        unsafe {
            let module = GetModuleHandleW(None).context("cannot find the executable resources")?;
            let small = LoadImageW(
                Some(module.into()),
                RESOURCE,
                IMAGE_ICON,
                GetSystemMetricsForDpi(SM_CXSMICON, dpi),
                GetSystemMetricsForDpi(SM_CYSMICON, dpi),
                LR_DEFAULTCOLOR,
            )
            .context("cannot load the small icon")?;
            next.small = Some(HICON(small.0));
            let large = LoadImageW(
                Some(module.into()),
                RESOURCE,
                IMAGE_ICON,
                GetSystemMetricsForDpi(SM_CXICON, dpi),
                GetSystemMetricsForDpi(SM_CYICON, dpi),
                LR_DEFAULTCOLOR,
            )
            .context("cannot load the large icon")?;
            next.large = Some(HICON(large.0));
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(small.0 as isize)),
            );
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(large.0 as isize)),
            );
        }
        *self = next;
        Ok(())
    }
}

impl Drop for Icons {
    fn drop(&mut self) {
        for icon in [self.small.take(), self.large.take()].into_iter().flatten() {
            let _ = unsafe { DestroyIcon(icon) };
        }
    }
}

/// Gives the window separate small and large icons and reloads them when its DPI changes.
pub fn install(handle: *mut c_void) {
    let hwnd = HWND(handle);
    let data = Box::into_raw(Box::<Icons>::default());
    unsafe {
        if !SetWindowSubclass(hwnd, Some(subclass), SUBCLASS, data as usize).as_bool() {
            drop(Box::from_raw(data));
            log::warn!("icon: cannot listen for display scale changes");
            return;
        }
        (*data).refresh(hwnd, GetDpiForWindow(hwnd));
    }
}

/// Tracks monitor and system metric changes without taking over GPUI's window procedure.
unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        match message {
            WM_DPICHANGED => (*(data as *mut Icons)).refresh(hwnd, (wparam.0 & 0xffff) as u32),
            WM_SETTINGCHANGE => (*(data as *mut Icons)).refresh(hwnd, GetDpiForWindow(hwnd)),
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS);
                let result = DefSubclassProc(hwnd, message, wparam, lparam);
                drop(Box::from_raw(data as *mut Icons));
                return result;
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}
