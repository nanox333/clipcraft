//! Windows wakes sleeping threads on a 15.6 ms clock unless a program asks for a finer one. A
//! thread that sleeps 5 ms then sleeps 15 ms, which turns a 60 frames-per-second playback into 30
//! and makes frame pacing in a recording drift. While a recording or a clip is running the program
//! asks for 1 ms; the rest of the time it leaves the system clock alone to save power.
//!
//! Only Windows is handled. The one `unsafe` call pair is documented where it is made.
#![cfg_attr(windows, allow(unsafe_code))]

/// Asks for a 1 ms timer clock (`true`) or gives the request back (`false`). Safe to call every
/// frame: only a change does anything.
#[cfg(windows)]
pub fn set_fine_timer(on: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
    static HELD: AtomicBool = AtomicBool::new(false);
    if HELD.swap(on, Ordering::SeqCst) == on {
        return;
    }
    // SAFETY: plain calls with a millisecond count. Each successful `timeBeginPeriod` is balanced
    // by one `timeEndPeriod` with the same value, because `HELD` only flips once per change.
    unsafe {
        if on {
            let _ = timeBeginPeriod(1);
        } else {
            let _ = timeEndPeriod(1);
        }
    }
}

/// Other systems keep their clock.
#[cfg(not(windows))]
pub fn set_fine_timer(on: bool) {
    let _ = on;
}
