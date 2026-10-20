//! Screen capture on Wayland: the desktop's own chooser (the ScreenCast portal) picks the screen
//! or window, and PipeWire carries the pictures. See `docs/wayland.md`.

mod pipewire;
mod pod;
mod portal;

use std::time::{Duration, Instant};

use clipcraft_capture::{
    Capabilities, CaptureError, CaptureFrame, CaptureSource, CaptureTarget, TargetKind,
};
use clipcraft_time::Tick;

/// The id of the one choice every Wayland computer offers: let the desktop ask.
pub const PORTAL_TARGET: &str = "wayland:portal";
/// How long to wait for the first picture after a screen is chosen.
const FIRST_PICTURE: Duration = Duration::from_secs(10);
/// How long `capture` waits for a new picture before repeating the last one (a still screen
/// sends nothing).
const FRAME_WAIT: Duration = Duration::from_millis(40);

/// Whether this session is Wayland and can capture through the portal.
pub fn available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|name| !name.is_empty())
            && pipewire::available()
            && portal::available()
    })
}

/// What is missing for Wayland capture, for a diagnostics report.
pub fn diagnose() -> (String, bool, String) {
    let name = "Wayland screen sharing".to_owned();
    if !pipewire::available() {
        return (
            name,
            false,
            "PipeWire's client library (libpipewire-0.3) is not installed".to_owned(),
        );
    }
    if !portal::available() {
        return (
            name,
            false,
            "the desktop does not offer the screen-cast portal (install xdg-desktop-portal and your desktop's backend)".to_owned(),
        );
    }
    (
        name,
        true,
        "the desktop will ask which screen or window to share".to_owned(),
    )
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        windows: true,
        cursor: true,
        exclusive_fullscreen: false,
        hdr: false,
    }
}

/// One entry: the desktop shows its own screen and window chooser when capture starts.
pub fn list_targets() -> Vec<CaptureTarget> {
    vec![CaptureTarget {
        id: PORTAL_TARGET.to_owned(),
        name: "Screen or window (your desktop will ask)".to_owned(),
        kind: TargetKind::Display,
        width: 0,
        height: 0,
    }]
}

pub fn open(show_cursor: bool) -> Result<Box<dyn CaptureSource>, CaptureError> {
    let connection = portal::session_bus().ok_or(CaptureError::NoProvider)?;
    open_on(connection, show_cursor)
}

/// Like [`open`], on the session bus at `address`. For tests.
pub fn open_at(address: &str, show_cursor: bool) -> Result<Box<dyn CaptureSource>, CaptureError> {
    let connection = portal::bus_at(address).ok_or(CaptureError::NoProvider)?;
    open_on(connection, show_cursor)
}

fn open_on(
    connection: zbus::blocking::Connection,
    show_cursor: bool,
) -> Result<Box<dyn CaptureSource>, CaptureError> {
    let share = portal::start(connection, show_cursor)?;
    let stream = pipewire::Stream::open(
        share.remote.try_clone().map_err(|_| CaptureError::Failed)?,
        share.node,
        FIRST_PICTURE,
    )?;
    let (width, height) = share.size.unwrap_or((0, 0));
    Ok(Box::new(WaylandSource {
        target: CaptureTarget {
            id: PORTAL_TARGET.to_owned(),
            name: share.name.clone().unwrap_or_else(|| "Screen".to_owned()),
            kind: TargetKind::Display,
            width,
            height,
        },
        stream,
        started: Instant::now(),
        _share: share,
    }))
}

struct WaylandSource {
    target: CaptureTarget,
    // Declared before `_share` so the stream stops before the portal session closes.
    stream: pipewire::Stream,
    started: Instant,
    _share: portal::Share,
}

impl CaptureSource for WaylandSource {
    fn target(&self) -> &CaptureTarget {
        &self.target
    }

    fn capture(&mut self) -> Result<CaptureFrame, CaptureError> {
        let (picture, _fresh) = self.stream.next_picture(FRAME_WAIT)?;
        self.target.width = picture.width;
        self.target.height = picture.height;
        let frame = CaptureFrame {
            width: picture.width,
            height: picture.height,
            stride: picture.width as usize * 4,
            bgra: picture.bgra,
            captured_at: Tick::from_units(
                i64::try_from(self.started.elapsed().as_micros()).unwrap_or(i64::MAX),
                1_000_000,
            ),
        };
        frame.validate()?;
        Ok(frame)
    }
}
