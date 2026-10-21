//! Operating-system capture providers and global hotkeys, behind a safe, `Result`-returning
//! interface (layer 5).
//!
//! * Linux (X11): screens and windows through the core protocol. Verified here under a virtual
//!   display.
//! * Linux (Wayland): the desktop's ScreenCast portal chooses the screen or window and PipeWire
//!   carries the pictures. See `wayland` and `docs/wayland.md`.
//! * Windows: Windows Graphics Capture, with generated-window AMD hardware evidence. See
//!   `docs/windows-performance.md` for measured limits.
//! * macOS: no provider yet.

use clipcraft_capture::{Capabilities, CaptureError, CaptureSource, CaptureTarget};

#[cfg(windows)]
mod d3d11;
#[cfg(any(target_os = "linux", test))]
mod h264_headers;
#[cfg(any(windows, target_os = "linux", test))]
mod h264_stream;
mod hardware;
mod hotkey;
#[cfg(windows)]
mod mf_encoder;
#[cfg(windows)]
mod mf_events;
mod priority;
mod process;
#[cfg(target_os = "linux")]
mod pulse;
#[cfg(target_os = "linux")]
mod vaapi;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(windows)]
mod windows;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;

pub use hardware::{hardware_provider, probe_hardware_encoder};
pub use hotkey::{HotkeyBinding, HotkeyEvent, HotkeyListener, parse_shortcut};
pub use priority::set_background_priority;
pub use process::{ProcessObserver, RunningProcess, running_processes};

/// Process-local counters for measuring the Windows native path without media/path logging.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapturePipelineStats {
    pub gpu_copies: u64,
    pub cpu_readbacks: u64,
    pub native_encoder_inputs: u64,
    pub cpu_encoder_inputs: u64,
    pub pool_drops: u64,
    pub feed_us: u64,
    pub conversion_us: u64,
    pub output_us: u64,
    pub input_wait_us: u64,
    pub input_submit_us: u64,
    pub event_us: u64,
}

/// Cumulative counters; compare snapshots around a recording when other sources are inactive.
pub fn capture_pipeline_stats() -> CapturePipelineStats {
    #[cfg(windows)]
    {
        d3d11::stats()
    }
    #[cfg(not(windows))]
    {
        CapturePipelineStats::default()
    }
}
#[cfg(target_os = "linux")]
pub use pulse::{Output, Outputs, list_outputs, open_system_monitor};
#[cfg(target_os = "linux")]
pub use wayland::PORTAL_TARGET;

/// What this computer's provider can do.
/// How a capture should look.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureOptions {
    /// Draw the mouse cursor in the picture, where the provider can.
    pub show_cursor: bool,
    /// Convert HDR to standard colours, where the provider can.
    pub hdr_to_sdr: bool,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            show_cursor: true,
            hdr_to_sdr: true,
        }
    }
}

pub fn capabilities() -> Capabilities {
    #[cfg(target_os = "linux")]
    if wayland::available() {
        return wayland::capabilities();
    }
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
    #[cfg(target_os = "linux")]
    if wayland::available() {
        return Ok(wayland::list_targets());
    }
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
    #[cfg(target_os = "linux")]
    if target_id == wayland::PORTAL_TARGET {
        return wayland::open(options.show_cursor);
    }
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

/// Like [`open`] for a Wayland screen share, with the portal on the session bus at `address`.
/// For tests, which stand in a fake desktop.
#[cfg(target_os = "linux")]
pub fn open_wayland_on(
    address: &str,
    options: &CaptureOptions,
) -> Result<Box<dyn CaptureSource>, CaptureError> {
    wayland::open_at(address, options.show_cursor)
}

/// The window that has the focus right now, or `None` when there is none or it cannot be
/// captured. Used by "force window capture" to find the game.
pub fn active_window() -> Result<Option<CaptureTarget>, CaptureError> {
    // Wayland does not tell other programs which window has the focus.
    #[cfg(target_os = "linux")]
    if wayland::available() {
        return Ok(None);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        x11::active_window(None)
    }
    #[cfg(windows)]
    {
        windows::active_window()
    }
    #[cfg(not(any(all(unix, not(target_os = "macos")), windows)))]
    {
        Err(CaptureError::NoProvider)
    }
}

/// Finds a capturable window owned by this process. Returns `None` where the desktop does not
/// expose window ownership (for example, a Wayland session using the screen-capture portal).
pub fn window_for_process(process_id: u32) -> Result<Option<CaptureTarget>, CaptureError> {
    #[cfg(target_os = "linux")]
    if wayland::available() {
        let _ = process_id;
        return Ok(None);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        x11::window_for_process(None, process_id)
    }
    #[cfg(windows)]
    {
        windows::window_for_process(process_id)
    }
    #[cfg(not(any(all(unix, not(target_os = "macos")), windows)))]
    {
        let _ = process_id;
        Ok(None)
    }
}

/// Like [`active_window`], on an explicit X display. For tests.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn active_window_on(display: &str) -> Result<Option<CaptureTarget>, CaptureError> {
    x11::active_window(Some(display))
}

/// Like [`window_for_process`], on an explicit X display. For tests.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn window_for_process_on(
    display: &str,
    process_id: u32,
) -> Result<Option<CaptureTarget>, CaptureError> {
    x11::window_for_process(Some(display), process_id)
}

/// One line of a diagnostics report: what was checked, whether it is fine, and the details.
pub type Finding = (String, bool, String);

/// What screen capture can do on this computer, and what it can see right now. Meant for a
/// diagnostics report; nothing is started.
pub fn diagnose_capture() -> Vec<Finding> {
    let mut findings = Vec::new();
    let capabilities = capabilities();
    findings.push((
        "Capabilities".to_owned(),
        true,
        format!(
            "windows {}, cursor {}, exclusive full screen {}, HDR conversion {}",
            capabilities.windows,
            capabilities.cursor,
            capabilities.exclusive_fullscreen,
            capabilities.hdr
        ),
    ));
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|name| !name.is_empty()) {
        findings.push(wayland::diagnose());
    }
    findings.push(match list_targets() {
        Ok(targets) => {
            let monitors = targets
                .iter()
                .filter(|target| target.kind == clipcraft_capture::TargetKind::Display)
                .count();
            let windows = targets.len() - monitors;
            (
                "Monitors and windows".to_owned(),
                monitors > 0,
                format!("{monitors} monitor(s) and {windows} window(s) can be captured"),
            )
        }
        Err(error) => ("Monitors and windows".to_owned(), false, error.to_string()),
    });
    findings
}

/// Which video encoders work here, by trying the hardware one briefly.
pub fn diagnose_video() -> Vec<Finding> {
    let mut findings = vec![(
        "Processor encoder".to_owned(),
        true,
        "built in (H.264), always available".to_owned(),
    )];
    findings.push(match probe_hardware_encoder() {
        Ok(name) => (
            "Graphics card encoder".to_owned(),
            true,
            format!("{name} works"),
        ),
        Err(why) => (
            "Graphics card encoder".to_owned(),
            true,
            format!("not available ({why}); recording uses the processor"),
        ),
    });
    findings
}

/// Whether what the computer is playing can be recorded, by opening that source for a moment.
pub fn diagnose_system_sound() -> Finding {
    #[cfg(target_os = "linux")]
    {
        match open_system_monitor("") {
            Ok(_) => (
                "Game and system sound".to_owned(),
                true,
                "the PulseAudio/PipeWire monitor of the default output can be recorded".to_owned(),
            ),
            Err(why) => ("Game and system sound".to_owned(), false, why),
        }
    }
    #[cfg(windows)]
    {
        (
            "Game and system sound".to_owned(),
            true,
            "recorded by loopback of the default output device".to_owned(),
        )
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        (
            "Game and system sound".to_owned(),
            false,
            "this system cannot record what it is playing".to_owned(),
        )
    }
}
