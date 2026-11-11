//! Single-instance guard via a named Windows mutex, called through
//! direct FFI so the window needs no extra crates — the same style as
//! the DPAPI module. Two instances sharing one data folder cannot
//! corrupt state (saves are atomic), but the last writer would win
//! and silently drop the other instance's edits, so a second launch
//! steps aside instead: the first instance (window or tray) is the
//! one to use.
//!
//! The mutex lives in the `Local\` namespace — one per logon session,
//! which is exactly the scope where the data folder is shared.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(attrs: *mut c_void, initial_owner: i32, name: *const u16) -> isize;
        fn GetLastError() -> u32;
        fn CloseHandle(handle: isize) -> i32;
    }
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    const ERROR_ALREADY_EXISTS: u32 = 183;
    const MB_OK: u32 = 0x0;
    const MB_ICONINFORMATION: u32 = 0x40;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Create (or open) the named mutex. Returns the handle to keep
    /// open for the process lifetime, or None when another instance
    /// already holds it. A null handle (creation failed outright)
    /// maps to Some(0): enforcement is best-effort and must never
    /// block startup.
    pub fn acquire(name: &str) -> Option<isize> {
        let wname = wide(name);
        let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 0, wname.as_ptr()) };
        if handle == 0 {
            return Some(0);
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return None;
        }
        Some(handle)
    }

    pub fn notify(text: &str, caption: &str) {
        let wtext = wide(text);
        let wcaption = wide(caption);
        unsafe {
            MessageBoxW(
                0,
                wtext.as_ptr(),
                wcaption.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }
}

const MUTEX_NAME: &str = "Local\\RepoStacks.SingleInstance.v1";

/// The held guard. Dropping it (process exit) releases the mutex;
/// keep it bound for all of `main`.
pub struct Guard {
    #[cfg(windows)]
    _handle: isize,
}

/// Try to become the one running instance. `None` means another
/// instance is already running and this process should exit.
pub fn acquire() -> Option<Guard> {
    #[cfg(windows)]
    {
        imp::acquire(MUTEX_NAME).map(|handle| Guard { _handle: handle })
    }
    #[cfg(not(windows))]
    {
        Some(Guard {})
    }
}

/// Tell the user why a second launch is stepping aside.
pub fn notify_already_running() {
    #[cfg(windows)]
    {
        imp::notify(
            "RepoStacks is already running. Check the window or the system tray — a second copy would fight over the same library.",
            "RepoStacks",
        );
    }
    #[cfg(not(windows))]
    {
        eprintln!("RepoStacks is already running.");
    }
}
