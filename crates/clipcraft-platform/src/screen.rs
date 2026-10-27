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

#[cfg(not(windows))]
pub fn make_window_passive(title: &str) -> bool {
    let _ = title;
    false
}
