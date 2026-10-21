//! The capture model: what can be captured, what a captured picture is, and the trait every
//! platform provider implements.
//!
//! This crate has no operating-system code. Providers (Windows Graphics Capture, X11, and others)
//! live in `clipcraft-platform` and hand back [`CaptureFrame`]s through [`CaptureSource`]. A
//! deterministic [`SyntheticSource`] stands in for hardware in tests.

pub mod hdr;
pub mod reframe;

use clipcraft_time::Tick;
use std::any::Any;
use std::fmt;

/// Widest or tallest picture ClipCraft will accept from a provider, in pixels.
pub const MAX_CAPTURE_EDGE: u32 = 7_680;

/// What kind of thing a target is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetKind {
    /// A whole monitor.
    Display,
    /// One application window.
    Window,
}

/// Something that can be captured.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureTarget {
    /// A provider-specific identifier, stable while the target exists.
    pub id: String,
    /// The name shown to people.
    pub name: String,
    /// Monitor or window.
    pub kind: TargetKind,
    /// Current width in pixels.
    pub width: u32,
    /// Current height in pixels.
    pub height: u32,
}

/// What a provider can do on this computer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    /// Can capture a single window, not only a monitor.
    pub windows: bool,
    /// Can include the mouse cursor in the picture.
    pub cursor: bool,
    /// Can capture a game running in exclusive full screen.
    pub exclusive_fullscreen: bool,
    /// Can convert HDR to standard colours.
    pub hdr: bool,
}

/// One captured picture: tightly described BGRA pixels, 8 bits per channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes from the start of one row to the next; at least `width * 4`.
    pub stride: usize,
    /// Pixel bytes in blue, green, red, alpha order.
    pub bgra: Vec<u8>,
    /// When the picture was taken, on the capture clock that starts at zero with the source.
    pub captured_at: Tick,
}

impl CaptureFrame {
    /// Checks that the dimensions, stride, and byte count agree, so later code can index safely.
    pub fn validate(&self) -> Result<(), CaptureError> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_CAPTURE_EDGE
            || self.height > MAX_CAPTURE_EDGE
        {
            return Err(CaptureError::BadFrame);
        }
        let row = (self.width as usize)
            .checked_mul(4)
            .ok_or(CaptureError::BadFrame)?;
        let needed = self
            .stride
            .checked_mul(self.height as usize - 1)
            .and_then(|bytes| bytes.checked_add(row))
            .ok_or(CaptureError::BadFrame)?;
        if self.stride < row || self.bgra.len() < needed {
            return Err(CaptureError::BadFrame);
        }
        Ok(())
    }
}

/// A platform-owned capture surface that can stay on its GPU until an encoder consumes it.
///
/// The concrete type and operating-system handles stay in `clipcraft-platform`. The recorder can
/// offer the opaque surface to a compatible hardware encoder, or request a CPU readback when a
/// surface/encoder pair is not supported. Implementations must keep any borrowed platform buffer
/// alive until this value is dropped.
pub trait NativeCaptureFrame: Send {
    /// Width in pixels.
    fn width(&self) -> u32;

    /// Height in pixels.
    fn height(&self) -> u32;

    /// Copies this surface to a CPU-owned frame for compatibility paths and screenshots.
    fn readback(&mut self) -> Result<CaptureFrame, CaptureError>;

    /// Lets the platform encoder recognize its own opaque surface type.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Why capture failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureError {
    /// This computer has no capture provider.
    NoProvider,
    /// The monitor or window was closed, unplugged, or became unavailable.
    TargetGone,
    /// The system refused access, for example a permission prompt was declined.
    Denied,
    /// The provider returned a picture with impossible dimensions or too few bytes.
    BadFrame,
    /// The provider failed in a way it could not describe further.
    Failed,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoProvider => "Capturing is not supported on this computer yet.",
            Self::TargetGone => {
                "The monitor or window being recorded went away. Pick it again to keep recording."
            }
            Self::Denied => "The system did not allow ClipCraft to capture the monitor.",
            Self::BadFrame => "The capture returned a damaged picture.",
            Self::Failed => "Capturing stopped working.",
        })
    }
}

impl std::error::Error for CaptureError {}

/// A running capture of one target. Providers implement this; the recorder polls it.
pub trait CaptureSource: Send {
    /// The target being captured, with its latest known size.
    fn target(&self) -> &CaptureTarget;

    /// Takes a native surface when this provider and a GPU encoder can share one.
    ///
    /// Existing providers keep returning `None`; the recorder then calls [`Self::capture`] as it
    /// always has. A native frame must own a bounded lease and provide a CPU readback fallback.
    fn capture_surface(&mut self) -> Result<Option<Box<dyn NativeCaptureFrame>>, CaptureError> {
        Ok(None)
    }

    /// Takes the current picture. May block briefly (up to one frame time) waiting for a new one.
    fn capture(&mut self) -> Result<CaptureFrame, CaptureError>;
}

/// A deterministic source for tests: a moving gradient whose every frame differs, optionally
/// failing after a fixed number of frames to simulate losing the device.
#[derive(Clone, Debug)]
pub struct SyntheticSource {
    target: CaptureTarget,
    fps: u32,
    next: u64,
    fail_after: Option<u64>,
    resize_after: Option<(u64, u32, u32)>,
}

impl SyntheticSource {
    /// A synthetic monitor of `width` x `height` whose clock advances `1 / fps` per frame.
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Self {
            target: CaptureTarget {
                id: "synthetic".to_owned(),
                name: "Test pattern".to_owned(),
                kind: TargetKind::Display,
                width,
                height,
            },
            fps: fps.max(1),
            next: 0,
            fail_after: None,
            resize_after: None,
        }
    }

    /// Makes `capture` return [`CaptureError::TargetGone`] after `frames` frames.
    pub fn failing_after(mut self, frames: u64) -> Self {
        self.fail_after = Some(frames);
        self
    }

    /// Changes the picture size after `frames` frames, as when a window is resized.
    pub fn resizing_after(mut self, frames: u64, width: u32, height: u32) -> Self {
        self.resize_after = Some((frames, width, height));
        self
    }

    /// The brightness the synthetic picture has at pixel column `x`, row `y` of frame `index`.
    pub fn sample(index: u64, x: u32, y: u32) -> [u8; 3] {
        let phase = (index % 256) as u32;
        [
            ((x + phase) % 256) as u8,
            ((y * 2 + phase) % 256) as u8,
            ((x + y + phase * 3) % 256) as u8,
        ]
    }
}

impl CaptureSource for SyntheticSource {
    fn target(&self) -> &CaptureTarget {
        &self.target
    }

    fn capture(&mut self) -> Result<CaptureFrame, CaptureError> {
        if self.fail_after.is_some_and(|limit| self.next >= limit) {
            return Err(CaptureError::TargetGone);
        }
        if let Some((frames, width, height)) = self.resize_after
            && self.next >= frames
        {
            self.target.width = width;
            self.target.height = height;
        }
        let (width, height) = (self.target.width, self.target.height);
        let stride = (width as usize)
            .checked_mul(4)
            .ok_or(CaptureError::BadFrame)?;
        let size = stride
            .checked_mul(height as usize)
            .filter(|size| *size <= 512 * 1024 * 1024)
            .ok_or(CaptureError::BadFrame)?;
        let mut bgra = vec![255_u8; size];
        for y in 0..height {
            let row = y as usize * stride;
            for x in 0..width {
                let [b, g, r] = Self::sample(self.next, x, y);
                let at = row + x as usize * 4;
                if let Some(pixel) = bgra.get_mut(at..at + 4) {
                    pixel.copy_from_slice(&[b, g, r, 255]);
                }
            }
        }
        let frame = CaptureFrame {
            width,
            height,
            stride,
            bgra,
            captured_at: Tick::from_units(
                i64::try_from(self.next).unwrap_or(i64::MAX),
                i64::from(self.fps),
            ),
        };
        self.next += 1;
        frame.validate()?;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_frames_differ_and_advance_the_clock() {
        let mut source = SyntheticSource::new(16, 8, 30);
        let first = source.capture().unwrap();
        let second = source.capture().unwrap();
        assert_ne!(first.bgra, second.bgra);
        assert_eq!(first.captured_at, Tick::from_units(0, 30));
        assert_eq!(second.captured_at, Tick::from_units(1, 30));
        first.validate().unwrap();
    }

    #[test]
    fn a_failing_source_reports_the_target_gone() {
        let mut source = SyntheticSource::new(4, 4, 30).failing_after(2);
        assert!(source.capture().is_ok() && source.capture().is_ok());
        assert_eq!(source.capture(), Err(CaptureError::TargetGone));
    }

    #[test]
    fn a_resizing_source_changes_the_frame_size() {
        let mut source = SyntheticSource::new(8, 8, 30).resizing_after(1, 16, 4);
        assert_eq!(source.capture().unwrap().width, 8);
        let resized = source.capture().unwrap();
        assert_eq!((resized.width, resized.height), (16, 4));
    }

    #[test]
    fn damaged_frames_are_rejected() {
        let good = CaptureFrame {
            width: 4,
            height: 2,
            stride: 16,
            bgra: vec![0; 32],
            captured_at: Tick::from_units(0, 30),
        };
        assert!(good.validate().is_ok());
        for bad in [
            CaptureFrame {
                width: 0,
                ..good.clone()
            },
            CaptureFrame {
                height: 0,
                ..good.clone()
            },
            CaptureFrame {
                width: MAX_CAPTURE_EDGE + 1,
                ..good.clone()
            },
            CaptureFrame {
                stride: 8,
                ..good.clone()
            },
            CaptureFrame {
                bgra: vec![0; 31],
                ..good.clone()
            },
            CaptureFrame {
                stride: usize::MAX,
                ..good.clone()
            },
        ] {
            assert_eq!(bad.validate(), Err(CaptureError::BadFrame));
        }
    }
}
