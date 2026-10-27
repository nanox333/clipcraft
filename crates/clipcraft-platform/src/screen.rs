//! Small facts and tweaks about the desktop that a pop-up notification needs: where the free part
//! of the screen is, and a way to show a window without taking the keyboard focus from a game.
//!
//! Only Windows is handled. The `unsafe` calls are documented where they are made.
#![cfg_attr(windows, allow(unsafe_code))]

/// The part of the main screen not covered by the taskbar, as `(left, top, right, bottom)` in
/// pixels, or `None` where it cannot be found.
#[cfg(windows)]
pub fn work_area() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    let mut area = RECT::default();
    // SAFETY: SPI_GETWORKAREA writes one RECT through the pointer, which is to a live local.
    unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&raw mut area).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()?;
    (area.right > area.left && area.bottom > area.top).then_some((
        area.left,
        area.top,
        area.right,
        area.bottom,
    ))
}

#[cfg(not(windows))]
pub fn work_area() -> Option<(i32, i32, i32, i32)> {
    None
}

/// Marks the window with this exact title as one that is never given the keyboard focus and has no
/// taskbar button, so showing it cannot pull a game out of the foreground. True when found.
#[cfg(windows)]
pub fn make_window_passive(title: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    };
    use windows::core::PCWSTR;
    let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a zero-terminated string that outlives the call.
    let Ok(window) = (unsafe { FindWindowW(PCWSTR::null(), PCWSTR(wide.as_ptr())) }) else {
        return false;
    };
    // SAFETY: `window` was just returned by the system; these read and write only its extended
    // style bits, and a window that has since closed makes the calls fail harmlessly.
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        let wanted = style | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize;
        if wanted != style {
            SetWindowLongPtrW(window, GWL_EXSTYLE, wanted);
        }
    }
    true
}

/// This process's top-level window with this exact title. Looked up by process so a second
/// ClipCraft (or any other program with the same title) is never touched.
#[cfg(windows)]
fn own_window(title: &str) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId,
    };
    use windows::core::BOOL;
    struct Search {
        title: Vec<u16>,
        process: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `Search` passed to `EnumWindows` below, alive for the whole call.
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let mut process = 0_u32;
        // SAFETY: `window` comes from the enumeration; `process` is a valid out pointer.
        unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
        if process == search.process {
            let mut text = [0_u16; 128];
            // SAFETY: `text` is a valid buffer of the length given.
            let length = unsafe { GetWindowTextW(window, &mut text) };
            let length = usize::try_from(length).unwrap_or(0);
            if text.get(..length) == Some(search.title.as_slice()) {
                search.found = Some(window);
                return BOOL(0);
            }
        }
        BOOL(1)
    }
    let mut search = Search {
        title: title.encode_utf16().collect(),
        // SAFETY: reads the calling process's id.
        process: unsafe { GetCurrentProcessId() },
        found: None,
    };
    // SAFETY: `visit` only uses `search` during this call, which does not outlive it.
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    search.found
}

/// Whether this process's window with this exact title is minimized.
#[cfg(windows)]
pub fn window_minimized(title: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsIconic;
    // SAFETY: `IsIconic` only reads the state of a window the system just returned.
    own_window(title).is_some_and(|window| unsafe { IsIconic(window).as_bool() })
}

#[cfg(not(windows))]
pub fn window_minimized(title: &str) -> bool {
    let _ = title;
    false
}

/// Shows or hides the taskbar button of the window with this exact title, leaving the window
/// itself (minimized or not) where it is. A minimized window with no button is reached from the
/// tray icon. True when the window was found.
#[cfg(windows)]
pub fn set_taskbar_button(title: &str, visible: bool) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, IsIconic, SW_HIDE, SW_SHOWMINNOACTIVE, SW_SHOWNA,
        SetWindowLongPtrW, ShowWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };
    let Some(window) = own_window(title) else {
        return false;
    };
    // SAFETY: `window` was just returned by the system; these read and write only its extended
    // style bits and its show state, and a window that has since closed makes the calls fail
    // harmlessly. The window is hidden and shown again so the taskbar notices the new style.
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        let wanted = if visible {
            (style & !(WS_EX_TOOLWINDOW.0 as isize)) | WS_EX_APPWINDOW.0 as isize
        } else {
            (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize)
        };
        if wanted != style {
            let minimized = IsIconic(window).as_bool();
            SetWindowLongPtrW(window, GWL_EXSTYLE, wanted);
            let _ = ShowWindow(window, SW_HIDE);
            let _ = ShowWindow(
                window,
                if minimized {
                    SW_SHOWMINNOACTIVE
                } else {
                    SW_SHOWNA
                },
            );
        }
    }
    true
}

#[cfg(not(windows))]
pub fn set_taskbar_button(title: &str, visible: bool) -> bool {
    let _ = (title, visible);
    false
}

#[cfg(not(windows))]
pub fn make_window_passive(title: &str) -> bool {
    let _ = title;
    false
}
