//! Lets the recorder yield to the game: while clipping, the whole program runs at a priority a
//! little below normal, so when the game and the encoder both want the processor the game gets it.
//!
//! Only Windows is handled. The one `unsafe` call is documented where it is made.
#![cfg_attr(windows, allow(unsafe_code))]

/// Runs this program at a lower priority (`true`) or back at the normal one (`false`). A failure
/// is ignored: the program just keeps the priority it had.
#[cfg(windows)]
pub fn set_background_priority(background: bool) {
    use windows::Win32::System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, NORMAL_PRIORITY_CLASS, SetPriorityClass,
    };
    let class = if background {
        BELOW_NORMAL_PRIORITY_CLASS
    } else {
        NORMAL_PRIORITY_CLASS
    };
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle for this process that needs no closing,
    // and the priority class is one of the documented constants.
    let _ = unsafe { SetPriorityClass(GetCurrentProcess(), class) };
}

/// Other systems keep their priority.
#[cfg(not(windows))]
pub fn set_background_priority(background: bool) {
    let _ = background;
}
