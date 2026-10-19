//! Operating-system capture providers and global hotkeys, behind a safe, `Result`-returning
//! interface (layer 5).
//!
//! * Linux (X11): screens and windows through the core protocol. Verified here under a virtual
//!   display. Wayland-only sessions have no provider yet (they need the desktop portal).
//! * Windows: Windows Graphics Capture. See the `windows` module; it compiles for Windows but has
//!   not been run on real hardware.
//! * macOS: no provider yet.

use clipcraft_capture::{Capabilities, CaptureError, CaptureSource, CaptureTarget};

mod hotkey;
#[cfg(windows)]
mod windows;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;

pub use hotkey::{HotkeyBinding, HotkeyEvent, HotkeyListener, parse_shortcut};

/// What this computer's provider can do.
/// How a capture should look.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureOptions {
    /// Draw the mouse cursor in the picture, where the provider can.
    pub show_cursor: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self { show_cursor: true }
    }
}

pub fn capabilities() -> Capabilities {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        x11::capabilities()
    }
    #[cfg(windows)]
    {
        windows::capabilities()
    }
    #[cfg(not(any(all(unix, not(target_os = "macos")), windows)))]
    {
        Capabilities::default()
    }
}

/// The monitors and windows that can be captured right now.
pub fn list_targets() -> Result<Vec<CaptureTarget>, CaptureError> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        x11::list_targets(None)
    }
    #[cfg(windows)]
    {
        windows::list_targets()
    }
    #[cfg(not(any(all(unix, not(target_os = "macos")), windows)))]
    {
        Err(CaptureError::NoProvider)
    }
}

/// Starts capturing the target with this id (one of [`list_targets`]).
pub fn open(
    target_id: &str,
    options: &CaptureOptions,
) -> Result<Box<dyn CaptureSource>, CaptureError> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = options;
        x11::open(None, target_id)
    }
    #[cfg(windows)]
    {
        windows::open(target_id, options)
    }
    #[cfg(not(any(all(unix, not(target_os = "macos")), windows)))]
    {
        let _ = (target_id, options);
        Err(CaptureError::NoProvider)
    }
}

/// Like [`list_targets`], on an explicit X display such as `:1`. For tests and remote sessions.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn list_targets_on(display: &str) -> Result<Vec<CaptureTarget>, CaptureError> {
    x11::list_targets(Some(display))
}

/// Like [`open`], on an explicit X display.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn open_on(display: &str, target_id: &str) -> Result<Box<dyn CaptureSource>, CaptureError> {
    x11::open(Some(display), target_id)
}
