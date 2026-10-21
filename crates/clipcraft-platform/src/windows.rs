//! Screen and window capture on Windows through the Windows Graphics Capture API, using the
//! `windows-capture` crate for WinRT and `d3d11` for bounded owned texture leases.
//!
//! Generated-window hardware evidence and remaining limitations are recorded in `docs/gaps.md`.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use clipcraft_capture::{
    Capabilities, CaptureError, CaptureFrame, CaptureSource, CaptureTarget, MAX_CAPTURE_EDGE,
    NativeCaptureFrame, TargetKind,
};
use clipcraft_time::Tick;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::CaptureOptions;

/// Most windows listed.
const MAX_WINDOWS: usize = 512;

pub fn capabilities() -> Capabilities {
    Capabilities {
        windows: true,
        cursor: true,
        // Games in exclusive full screen may not be capturable; borderless windowed always is.
        exclusive_fullscreen: false,
        hdr: true,
    }
}

pub fn list_targets() -> Result<Vec<CaptureTarget>, CaptureError> {
    let mut targets = Vec::new();
    let monitors = Monitor::enumerate().map_err(|_| CaptureError::Failed)?;
    for (index, monitor) in monitors.iter().enumerate() {
        let (Ok(width), Ok(height)) = (monitor.width(), monitor.height()) else {
            continue;
        };
        let name = monitor
            .name()
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("Display {}", index + 1));
        targets.push(CaptureTarget {
            id: format!("monitor:{index}"),
            name,
            kind: TargetKind::Display,
            width,
            height,
        });
    }
    for window in Window::enumerate()
        .map_err(|_| CaptureError::Failed)?
        .into_iter()
        .take(MAX_WINDOWS)
    {
        let (Ok(title), Ok(width), Ok(height)) = (window.title(), window.width(), window.height())
        else {
            continue;
        };
        if title.is_empty() || width <= 0 || height <= 0 {
            continue;
        }
        targets.push(CaptureTarget {
            id: format!("window:{:x}", window.as_raw_hwnd() as usize),
            name: title,
            kind: TargetKind::Window,
            width: width as u32,
            height: height as u32,
        });
    }
    Ok(targets)
}

/// Finds a capturable top-level window owned by `process_id`.
pub fn window_for_process(process_id: u32) -> Result<Option<CaptureTarget>, CaptureError> {
    let windows = Window::enumerate().map_err(|_| CaptureError::Failed)?;
    for window in windows.into_iter().take(MAX_WINDOWS) {
        if window.process_id().ok() != Some(process_id) {
            continue;
        }
        let (Ok(title), Ok(width), Ok(height)) = (window.title(), window.width(), window.height())
        else {
            continue;
        };
        if title.is_empty() || width <= 0 || height <= 0 || !window.is_valid() {
            continue;
        }
        return Ok(Some(CaptureTarget {
            id: format!("window:{:x}", window.as_raw_hwnd() as usize),
            name: title,
            kind: TargetKind::Window,
            width: width as u32,
            height: height as u32,
        }));
    }
    Ok(None)
}

/// The newest picture the capture thread produced.
#[derive(Default)]
struct Latest {
    frame: Option<CaptureFrame>,
    surface: Option<crate::d3d11::Surface>,
    sequence: u64,
    closed: bool,
    failed: bool,
}

struct Shared {
    latest: Mutex<Latest>,
    changed: Condvar,
    started: Instant,
}

fn lock(shared: &Shared) -> MutexGuard<'_, Latest> {
    shared.latest.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Handler {
    shared: Arc<Shared>,
    pool: crate::d3d11::Pool,
    native_disabled: bool,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Arc<Shared>;
    type Error = &'static str;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            shared: context.flags,
            pool: crate::d3d11::Pool::default(),
            native_disabled: false,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame<'_>,
        _control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let result = (|| {
            let captured_at = Tick::from_units(
                i64::try_from(self.shared.started.elapsed().as_micros()).unwrap_or(i64::MAX),
                1_000_000,
            );
            if frame.color_format() == ColorFormat::Bgra8 && !self.native_disabled {
                match self.pool.copy(frame, captured_at) {
                    Ok(Some(surface)) => {
                        let mut latest = lock(&self.shared);
                        latest.surface = Some(surface);
                        latest.frame = None;
                        latest.sequence = latest.sequence.wrapping_add(1);
                        drop(latest);
                        self.shared.changed.notify_all();
                        return Ok(());
                    }
                    // Every slot is leased. Keep the previous image and bound GPU memory.
                    Ok(None) => return Ok(()),
                    // The existing CPU path handles unsupported devices, formats and budgets.
                    Err(_) => self.native_disabled = true,
                }
            }
            let buffer = frame.buffer().map_err(|_| "could not read the frame")?;
            crate::d3d11::readback_count();
            let (width, height) = (buffer.width(), buffer.height());
            if width == 0 || height == 0 || width > MAX_CAPTURE_EDGE || height > MAX_CAPTURE_EDGE {
                return Err("impossible frame size");
            }
            let mut scratch = Vec::new();
            let pixels = buffer.as_nopadding_buffer(&mut scratch);
            // HDR frames arrive as linear half floats and are tone mapped to standard range here.
            let bgra = if buffer.color_format() == ColorFormat::Rgba16F {
                let white = clipcraft_capture::hdr::reference_white_for(pixels);
                clipcraft_capture::hdr::scrgb_to_bgra8(
                    pixels,
                    width,
                    height,
                    width as usize * 8,
                    white,
                )
                .ok_or("damaged HDR frame")?
            } else {
                pixels.to_vec()
            };
            let captured = CaptureFrame {
                width,
                height,
                stride: width as usize * 4,
                bgra,
                captured_at,
            };
            if captured.validate().is_err() {
                return Err("damaged frame");
            }
            let mut latest = lock(&self.shared);
            latest.frame = Some(captured);
            latest.surface = None;
            latest.sequence = latest.sequence.wrapping_add(1);
            drop(latest);
            self.shared.changed.notify_all();
            Ok(())
        })();
        if result.is_err() {
            lock(&self.shared).failed = true;
            self.shared.changed.notify_all();
        }
        result
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        lock(&self.shared).closed = true;
        self.shared.changed.notify_all();
        Ok(())
    }
}

struct WindowsSource {
    shared: Arc<Shared>,
    control: Option<CaptureControl<Handler, &'static str>>,
    target: CaptureTarget,
    seen: u64,
    pending_cpu: Option<CaptureFrame>,
}

fn settings<T: TryInto<GraphicsCaptureItemType>>(
    item: T,
    cursor: CursorCaptureSettings,
    color: ColorFormat,
    shared: &Arc<Shared>,
) -> Settings<Arc<Shared>, T> {
    Settings::new(
        item,
        cursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        color,
        shared.clone(),
    )
}

pub fn open(
    target_id: &str,
    options: &CaptureOptions,
) -> Result<Box<dyn CaptureSource>, CaptureError> {
    let shared = Arc::new(Shared {
        latest: Mutex::new(Latest::default()),
        changed: Condvar::new(),
        started: Instant::now(),
    });
    let cursor = if options.show_cursor {
        CursorCaptureSettings::WithCursor
    } else {
        CursorCaptureSettings::WithoutCursor
    };
    // Asking for half floats is how HDR is captured; on a standard display they hold the same
    // picture and the tone mapper leaves it alone.
    let color = if options.hdr_to_sdr {
        ColorFormat::Rgba16F
    } else {
        ColorFormat::Bgra8
    };
    let (control, target) = if let Some(index) = target_id.strip_prefix("monitor:") {
        let index: usize = index.parse().map_err(|_| CaptureError::TargetGone)?;
        let monitor = Monitor::from_index(index).map_err(|_| CaptureError::TargetGone)?;
        let (width, height) = (
            monitor.width().map_err(|_| CaptureError::Failed)?,
            monitor.height().map_err(|_| CaptureError::Failed)?,
        );
        let control = Handler::start_free_threaded(settings(monitor, cursor, color, &shared))
            .map_err(|_| CaptureError::Failed)?;
        (
            control,
            CaptureTarget {
                id: target_id.to_owned(),
                name: format!("Display {}", index + 1),
                kind: TargetKind::Display,
                width,
                height,
            },
        )
    } else if let Some(handle) = target_id.strip_prefix("window:") {
        let handle = usize::from_str_radix(handle, 16).map_err(|_| CaptureError::TargetGone)?;
        let window = Window::from_raw_hwnd(handle as *mut std::ffi::c_void);
        if !window.is_valid() {
            return Err(CaptureError::TargetGone);
        }
        let title = window.title().unwrap_or_else(|_| "Window".to_owned());
        let (width, height) = (
            u32::try_from(window.width().map_err(|_| CaptureError::TargetGone)?).unwrap_or(0),
            u32::try_from(window.height().map_err(|_| CaptureError::TargetGone)?).unwrap_or(0),
        );
        let control = Handler::start_free_threaded(settings(window, cursor, color, &shared))
            .map_err(|_| CaptureError::Failed)?;
        (
            control,
            CaptureTarget {
                id: target_id.to_owned(),
                name: title,
                kind: TargetKind::Window,
                width,
                height,
            },
        )
    } else {
        return Err(CaptureError::TargetGone);
    };
    Ok(Box::new(WindowsSource {
        shared,
        control: Some(control),
        target,
        seen: 0,
        pending_cpu: None,
    }))
}

impl CaptureSource for WindowsSource {
    fn target(&self) -> &CaptureTarget {
        &self.target
    }

    fn capture_surface(&mut self) -> Result<Option<Box<dyn NativeCaptureFrame>>, CaptureError> {
        match self.next_frame()? {
            Captured::Surface(surface) => Ok(Some(Box::new(surface))),
            Captured::Cpu(frame) => {
                self.pending_cpu = Some(frame);
                Ok(None)
            }
        }
    }

    fn capture(&mut self) -> Result<CaptureFrame, CaptureError> {
        if let Some(frame) = self.pending_cpu.take() {
            return Ok(frame);
        }
        match self.next_frame()? {
            Captured::Surface(mut surface) => surface.readback(),
            Captured::Cpu(frame) => Ok(frame),
        }
    }
}

enum Captured {
    Surface(crate::d3d11::Surface),
    Cpu(CaptureFrame),
}

impl WindowsSource {
    fn next_frame(&mut self) -> Result<Captured, CaptureError> {
        // Graphics Capture only delivers a picture when the screen changes, so wait a little for a
        // new one and otherwise repeat the last.
        let deadline = Instant::now() + Duration::from_millis(40);
        let mut latest = lock(&self.shared);
        while latest.sequence == self.seen && !latest.closed && !latest.failed {
            let now = Instant::now();
            // With no picture at all yet, allow a longer wait for the first one.
            let limit = if latest.frame.is_none() && latest.surface.is_none() {
                deadline + Duration::from_secs(3)
            } else {
                deadline
            };
            if now >= limit {
                break;
            }
            latest = self
                .shared
                .changed
                .wait_timeout(latest, limit - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        if latest.closed {
            return Err(CaptureError::TargetGone);
        }
        if latest.failed {
            return Err(CaptureError::Failed);
        }
        self.seen = latest.sequence;
        if let Some(surface) = &latest.surface {
            self.target.width = surface.width();
            self.target.height = surface.height();
            return Ok(Captured::Surface(surface.clone()));
        }
        let frame = latest.frame.clone().ok_or(CaptureError::Failed)?;
        self.target.width = frame.width;
        self.target.height = frame.height;
        Ok(Captured::Cpu(frame))
    }
}

impl Drop for WindowsSource {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            let _ = control.stop();
        }
    }
}

// ----- hotkeys -----

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::hotkey::{HotkeyBinding, HotkeyEvent, HotkeyListener, parse_shortcut};

/// Hotkeys registered with the operating system. Dropping it unregisters them.
pub struct WindowsHotkeys {
    manager: GlobalHotKeyManager,
    registered: Vec<(HotKey, u32)>,
}

impl WindowsHotkeys {
    /// The next press or release of a registered shortcut.
    pub fn try_next(&self) -> Option<HotkeyEvent> {
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if let Some((_, id)) = self.registered.iter().find(|(key, _)| key.id() == event.id) {
                return Some(HotkeyEvent {
                    id: *id,
                    pressed: event.state == HotKeyState::Pressed,
                });
            }
        }
        None
    }
}

impl Drop for WindowsHotkeys {
    fn drop(&mut self) {
        for (key, _) in &self.registered {
            let _ = self.manager.unregister(*key);
        }
    }
}

fn code_of(name: &str) -> Option<Code> {
    let mut chars = name.chars();
    if let (Some(only), None) = (chars.next(), chars.next()) {
        if only.is_ascii_uppercase() {
            return format!("Key{only}").parse().ok();
        }
        if only.is_ascii_digit() {
            return format!("Digit{only}").parse().ok();
        }
    }
    if name.starts_with('F') && name[1..].chars().all(|c| c.is_ascii_digit()) {
        return name.parse().ok();
    }
    let canonical = match name {
        "Space" | "Tab" | "Enter" | "Backspace" | "Delete" | "Home" | "End" | "PageUp"
        | "PageDown" | "Insert" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight"
        | "Minus" | "Comma" | "Period" | "Slash" => name,
        "Equals" => "Equal",
        "Backtick" => "Backquote",
        _ => return None,
    };
    canonical.parse().ok()
}

pub fn start_hotkeys(bindings: Vec<HotkeyBinding>) -> Result<HotkeyListener, CaptureError> {
    let manager = GlobalHotKeyManager::new().map_err(|_| CaptureError::Failed)?;
    let mut registered = Vec::new();
    let mut skipped = Vec::new();
    for binding in bindings {
        let key = parse_shortcut(&binding.shortcut).and_then(|(modifiers, name)| {
            let mut mods = Modifiers::empty();
            if modifiers.ctrl {
                mods |= Modifiers::CONTROL;
            }
            if modifiers.alt {
                mods |= Modifiers::ALT;
            }
            if modifiers.shift {
                mods |= Modifiers::SHIFT;
            }
            Some(HotKey::new(Some(mods), code_of(&name)?))
        });
        match key {
            Some(key) if manager.register(key).is_ok() => registered.push((key, binding.id)),
            _ => skipped.push(binding.shortcut),
        }
    }
    Ok(HotkeyListener::from_windows(
        WindowsHotkeys {
            manager,
            registered,
        },
        skipped,
    ))
}

/// The window in front, if there is one and it can be captured.
pub fn active_window() -> Result<Option<CaptureTarget>, CaptureError> {
    let Ok(window) = Window::foreground() else {
        return Ok(None);
    };
    let (Ok(title), Ok(width), Ok(height)) = (window.title(), window.width(), window.height())
    else {
        return Ok(None);
    };
    if title.is_empty() || width <= 0 || height <= 0 || !window.is_valid() {
        return Ok(None);
    }
    Ok(Some(CaptureTarget {
        id: format!("window:{:x}", window.as_raw_hwnd() as usize),
        name: title,
        kind: TargetKind::Window,
        width: width as u32,
        height: height as u32,
    }))
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    fn source(shared: Arc<Shared>) -> WindowsSource {
        WindowsSource {
            shared,
            control: None,
            seen: 0,
            pending_cpu: None,
            target: CaptureTarget {
                id: "generated".to_owned(),
                name: "generated".to_owned(),
                kind: TargetKind::Window,
                width: 1,
                height: 1,
            },
        }
    }

    #[test]
    fn failed_callback_wakes_capture_instead_of_leaving_it_running() {
        let shared = Arc::new(Shared {
            latest: Mutex::new(Latest::default()),
            changed: Condvar::new(),
            started: Instant::now(),
        });
        let mut source = source(Arc::clone(&shared));
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = send.send(source.capture().err());
        });
        lock(&shared).failed = true;
        shared.changed.notify_all();
        assert_eq!(
            receive.recv_timeout(Duration::from_secs(1)).unwrap(),
            Some(CaptureError::Failed)
        );
        worker.join().unwrap();
    }

    #[test]
    fn target_loss_takes_precedence_over_a_cached_picture() {
        let shared = Arc::new(Shared {
            latest: Mutex::new(Latest {
                frame: Some(CaptureFrame {
                    width: 1,
                    height: 1,
                    stride: 4,
                    bgra: vec![0; 4],
                    captured_at: Tick(0),
                }),
                sequence: 1,
                closed: true,
                ..Latest::default()
            }),
            changed: Condvar::new(),
            started: Instant::now(),
        });
        assert_eq!(
            source(shared).capture().err(),
            Some(CaptureError::TargetGone)
        );
    }
}
