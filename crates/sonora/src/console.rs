/// Lends this process the console of the terminal that started it, before the command line
/// prints. A Windows release build has no console of its own, and without one to borrow there
/// is nowhere to print, which is not an error. Elsewhere stdout is always there.
pub fn attach() {
    #[cfg(windows)]
    // SAFETY: AttachConsole takes no pointers and only reports whether a console was attached.
    unsafe {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
