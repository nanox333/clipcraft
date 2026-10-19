//! Screen, window, and hotkey access on X11 using the core protocol (no `unsafe`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clipcraft_capture::{
    Capabilities, CaptureError, CaptureFrame, CaptureSource, CaptureTarget, MAX_CAPTURE_EDGE,
    TargetKind,
};
use clipcraft_time::Tick;
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, GrabMode, ImageFormat, ImageOrder, ModMask, Window,
};
use x11rb::rust_connection::RustConnection;

use crate::hotkey::{HotkeyBinding, HotkeyEvent, HotkeyListener, parse_shortcut};

/// Most bytes asked of the server in one `GetImage`, so a huge screen is read in bands.
const BAND_BYTES: usize = 8 * 1024 * 1024;
/// Most windows listed.
const MAX_WINDOWS: usize = 512;

pub fn capabilities() -> Capabilities {
    Capabilities {
        windows: true,
        cursor: false,
        exclusive_fullscreen: false,
        hdr: false,
    }
}

fn connect(display: Option<&str>) -> Result<(RustConnection, usize), CaptureError> {
    RustConnection::connect(display).map_err(|_| CaptureError::NoProvider)
}

/// Where a target is on the X server.
#[derive(Clone, Copy, Debug)]
enum Place {
    Monitor {
        x: i16,
        y: i16,
        width: u16,
        height: u16,
    },
    Window(Window),
}

fn parse_id(id: &str) -> Option<Place> {
    if let Some(rest) = id.strip_prefix("window:") {
        return u32::from_str_radix(rest, 16).ok().map(Place::Window);
    }
    let rest = id.strip_prefix("monitor:")?;
    let mut parts = rest.split(',').map(str::parse::<i32>);
    let (x, y, width, height) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    if parts.next().is_some() {
        return None;
    }
    Some(Place::Monitor {
        x: i16::try_from(x).ok()?,
        y: i16::try_from(y).ok()?,
        width: u16::try_from(width).ok()?,
        height: u16::try_from(height).ok()?,
    })
}

fn atom(conn: &RustConnection, name: &[u8]) -> Option<Atom> {
    conn.intern_atom(false, name)
        .ok()?
        .reply()
        .ok()
        .map(|reply| reply.atom)
}

fn window_name(conn: &RustConnection, window: Window, net_name: Atom, utf8: Atom) -> String {
    let read = |property: Atom, kind: Atom| -> Option<String> {
        let reply = conn
            .get_property(false, window, property, kind, 0, 64)
            .ok()?
            .reply()
            .ok()?;
        let text = String::from_utf8_lossy(&reply.value).trim().to_owned();
        (!text.is_empty()).then_some(text)
    };
    read(net_name, utf8)
        .or_else(|| read(AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()))
        .unwrap_or_else(|| "Untitled window".to_owned())
}

pub fn list_targets(display: Option<&str>) -> Result<Vec<CaptureTarget>, CaptureError> {
    let (conn, screen_number) = connect(display)?;
    let screen = conn
        .setup()
        .roots
        .get(screen_number)
        .ok_or(CaptureError::NoProvider)?;
    let root = screen.root;
    let mut targets = Vec::new();

    // Monitors, from RandR when the server has it; otherwise the whole root window.
    let monitors = conn
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.monitors)
        .unwrap_or_default();
    if monitors.is_empty() {
        targets.push(CaptureTarget {
            id: format!(
                "monitor:0,0,{},{}",
                screen.width_in_pixels, screen.height_in_pixels
            ),
            name: "Screen".to_owned(),
            kind: TargetKind::Display,
            width: u32::from(screen.width_in_pixels),
            height: u32::from(screen.height_in_pixels),
        });
    }
    for (index, monitor) in monitors.iter().enumerate() {
        let name = conn
            .get_atom_name(monitor.name)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("Display {}", index + 1));
        targets.push(CaptureTarget {
            id: format!(
                "monitor:{},{},{},{}",
                monitor.x, monitor.y, monitor.width, monitor.height
            ),
            name: if monitor.primary {
                format!("{name} (main)")
            } else {
                name
            },
            kind: TargetKind::Display,
            width: u32::from(monitor.width),
            height: u32::from(monitor.height),
        });
    }

    // Windows the window manager lists.
    if let (Some(list), Some(net_name), Some(utf8)) = (
        atom(&conn, b"_NET_CLIENT_LIST"),
        atom(&conn, b"_NET_WM_NAME"),
        atom(&conn, b"UTF8_STRING"),
    ) && let Some(reply) = conn
        .get_property(false, root, list, AtomEnum::WINDOW, 0, MAX_WINDOWS as u32)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        && let Some(ids) = reply.value32()
    {
        for window in ids.take(MAX_WINDOWS) {
            let Some(geometry) = conn.get_geometry(window).ok().and_then(|c| c.reply().ok()) else {
                continue;
            };
            if geometry.width == 0 || geometry.height == 0 {
                continue;
            }
            targets.push(CaptureTarget {
                id: format!("window:{window:x}"),
                name: window_name(&conn, window, net_name, utf8),
                kind: TargetKind::Window,
                width: u32::from(geometry.width),
                height: u32::from(geometry.height),
            });
        }
    }
    Ok(targets)
}

struct X11Source {
    conn: RustConnection,
    root: Window,
    place: Place,
    target: CaptureTarget,
    started: Instant,
    last: Option<CaptureFrame>,
}

pub fn open(
    display: Option<&str>,
    target_id: &str,
) -> Result<Box<dyn CaptureSource>, CaptureError> {
    let place = parse_id(target_id).ok_or(CaptureError::TargetGone)?;
    let (conn, screen_number) = connect(display)?;
    let screen = conn
        .setup()
        .roots
        .get(screen_number)
        .ok_or(CaptureError::NoProvider)?;
    let root = screen.root;
    // Only the common 24-bit little-endian BGRX layout is read.
    let format_ok = conn.setup().image_byte_order == ImageOrder::LSB_FIRST
        && conn.setup().pixmap_formats.iter().any(|format| {
            format.depth == screen.root_depth
                && format.bits_per_pixel == 32
                && format.scanline_pad == 32
        })
        && screen.root_depth >= 24;
    if !format_ok {
        return Err(CaptureError::NoProvider);
    }
    let (width, height, kind, name) = match place {
        Place::Monitor { width, height, .. } => (
            u32::from(width),
            u32::from(height),
            TargetKind::Display,
            "Display".to_owned(),
        ),
        Place::Window(window) => {
            let geometry = conn
                .get_geometry(window)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .ok_or(CaptureError::TargetGone)?;
            (
                u32::from(geometry.width),
                u32::from(geometry.height),
                TargetKind::Window,
                "Window".to_owned(),
            )
        }
    };
    Ok(Box::new(X11Source {
        conn,
        root,
        place,
        target: CaptureTarget {
            id: target_id.to_owned(),
            name,
            kind,
            width,
            height,
        },
        started: Instant::now(),
        last: None,
    }))
}

fn classify(error: &ReplyError) -> CaptureError {
    use x11rb::protocol::ErrorKind;
    match error {
        ReplyError::X11Error(error) => match error.error_kind {
            ErrorKind::Window | ErrorKind::Drawable => CaptureError::TargetGone,
            ErrorKind::Access => CaptureError::Denied,
            _ => CaptureError::Failed,
        },
        ReplyError::ConnectionError(_) => CaptureError::Failed,
    }
}

impl X11Source {
    fn grab(&mut self) -> Result<CaptureFrame, CaptureError> {
        let (drawable, x, y, width, height) = match self.place {
            Place::Monitor {
                x,
                y,
                width,
                height,
            } => (self.root, x, y, u32::from(width), u32::from(height)),
            Place::Window(window) => {
                let geometry = self
                    .conn
                    .get_geometry(window)
                    .map_err(|_| CaptureError::Failed)?
                    .reply()
                    .map_err(|error| classify(&error))?;
                (
                    window,
                    0,
                    0,
                    u32::from(geometry.width),
                    u32::from(geometry.height),
                )
            }
        };
        if width == 0 || height == 0 || width > MAX_CAPTURE_EDGE || height > MAX_CAPTURE_EDGE {
            return Err(CaptureError::BadFrame);
        }
        let stride = width as usize * 4;
        let rows_per_band = (BAND_BYTES / stride).max(1) as u32;
        let mut bgra = Vec::with_capacity(stride * height as usize);
        let mut row = 0_u32;
        while row < height {
            let rows = rows_per_band.min(height - row);
            let reply = self
                .conn
                .get_image(
                    ImageFormat::Z_PIXMAP,
                    drawable,
                    x,
                    y.saturating_add(i16::try_from(row).map_err(|_| CaptureError::BadFrame)?),
                    u16::try_from(width).map_err(|_| CaptureError::BadFrame)?,
                    u16::try_from(rows).map_err(|_| CaptureError::BadFrame)?,
                    !0,
                )
                .map_err(|_| CaptureError::Failed)?
                .reply()
                .map_err(|error| classify(&error))?;
            if reply.data.len() != stride * rows as usize {
                return Err(CaptureError::BadFrame);
            }
            bgra.extend_from_slice(&reply.data);
            row += rows;
        }
        // The server leaves the fourth byte of each pixel undefined.
        for pixel in bgra.chunks_exact_mut(4) {
            if let Some(alpha) = pixel.get_mut(3) {
                *alpha = 255;
            }
        }
        self.target.width = width;
        self.target.height = height;
        Ok(CaptureFrame {
            width,
            height,
            stride,
            bgra,
            captured_at: Tick::from_units(
                i64::try_from(self.started.elapsed().as_micros()).unwrap_or(i64::MAX),
                1_000_000,
            ),
        })
    }
}

impl CaptureSource for X11Source {
    fn target(&self) -> &CaptureTarget {
        &self.target
    }

    fn capture(&mut self) -> Result<CaptureFrame, CaptureError> {
        match self.grab() {
            Ok(frame) => {
                self.last = Some(frame.clone());
                Ok(frame)
            }
            // A minimised or momentarily unmapped window cannot be read: show the last picture.
            Err(CaptureError::Failed) if matches!(self.place, Place::Window(_)) => {
                self.last.clone().ok_or(CaptureError::Failed)
            }
            Err(error) => Err(error),
        }
    }
}

// ----- hotkeys -----

fn keysym(name: &str) -> Option<u32> {
    let mut chars = name.chars();
    if let (Some(only), None) = (chars.next(), chars.next()) {
        if only.is_ascii_uppercase() {
            return Some(only.to_ascii_lowercase() as u32);
        }
        if only.is_ascii_digit() {
            return Some(only as u32);
        }
    }
    if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&number)
    {
        return Some(0xffbe + number - 1);
    }
    Some(match name {
        "Space" => 0x20,
        "Tab" => 0xff09,
        "Enter" => 0xff0d,
        "Backspace" => 0xff08,
        "Delete" => 0xffff,
        "Home" => 0xff50,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "End" => 0xff57,
        "Insert" => 0xff63,
        "Minus" => 0x2d,
        "Equals" => 0x3d,
        "Comma" => 0x2c,
        "Period" => 0x2e,
        "Slash" => 0x2f,
        "Backtick" => 0x60,
        _ => return None,
    })
}

struct Grab {
    id: u32,
    keycode: u8,
    state: u16,
}

pub fn start_hotkeys(
    display: Option<&str>,
    bindings: Vec<HotkeyBinding>,
) -> Result<HotkeyListener, CaptureError> {
    let (conn, screen_number) = connect(display)?;
    let root = conn
        .setup()
        .roots
        .get(screen_number)
        .ok_or(CaptureError::NoProvider)?
        .root;
    let min = conn.setup().min_keycode;
    let max = conn.setup().max_keycode;
    let mapping = conn
        .get_keyboard_mapping(min, max.saturating_sub(min).saturating_add(1))
        .map_err(|_| CaptureError::Failed)?
        .reply()
        .map_err(|_| CaptureError::Failed)?;
    let per = usize::from(mapping.keysyms_per_keycode).max(1);
    let keycode_of = |sym: u32| -> Option<u8> {
        mapping
            .keysyms
            .chunks(per)
            .position(|syms| syms.first() == Some(&sym))
            .and_then(|index| u8::try_from(index).ok())
            .and_then(|index| min.checked_add(index))
    };

    let mut grabs: Vec<Grab> = Vec::new();
    let mut skipped = Vec::new();
    for binding in &bindings {
        let registered = parse_shortcut(&binding.shortcut)
            .and_then(|(modifiers, key)| Some((modifiers, keysym(&key)?)))
            .and_then(|(modifiers, sym)| Some((modifiers, keycode_of(sym)?)));
        let Some((modifiers, keycode)) = registered else {
            skipped.push(binding.shortcut.clone());
            continue;
        };
        let mut base = ModMask::default();
        if modifiers.ctrl {
            base |= ModMask::CONTROL;
        }
        if modifiers.alt {
            base |= ModMask::M1;
        }
        if modifiers.shift {
            base |= ModMask::SHIFT;
        }
        // Also fire with Caps Lock and Num Lock on.
        let mut ok = true;
        for extra in [
            0_u16,
            u16::from(ModMask::LOCK),
            u16::from(ModMask::M2),
            u16::from(ModMask::LOCK) | u16::from(ModMask::M2),
        ] {
            let state = u16::from(base) | extra;
            let sent = conn
                .grab_key(
                    false,
                    root,
                    ModMask::from(state),
                    keycode,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .ok()
                .and_then(|cookie| cookie.check().ok());
            if sent.is_none() {
                ok = false;
                break;
            }
            grabs.push(Grab {
                id: binding.id,
                keycode,
                state,
            });
        }
        if !ok {
            skipped.push(binding.shortcut.clone());
        }
    }
    let _ = conn.flush();

    let (sender, receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    std::thread::Builder::new()
        .name("clipcraft-hotkeys".to_owned())
        .spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                match conn.poll_for_event() {
                    Ok(Some(event)) => {
                        let (keycode, state, pressed) = match event {
                            Event::KeyPress(key) => (key.detail, u16::from(key.state), true),
                            Event::KeyRelease(key) => (key.detail, u16::from(key.state), false),
                            _ => continue,
                        };
                        // Ignore which lock keys are on and any mouse-button bits.
                        let wanted = state & 0x0d;
                        if let Some(grab) = grabs
                            .iter()
                            .find(|grab| grab.keycode == keycode && grab.state & 0x0d == wanted)
                            && sender
                                .send(HotkeyEvent {
                                    id: grab.id,
                                    pressed,
                                })
                                .is_err()
                        {
                            break;
                        }
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
        })
        .map_err(|_| CaptureError::Failed)?;
    Ok(HotkeyListener::from_channel(receiver, stop, skipped))
}
