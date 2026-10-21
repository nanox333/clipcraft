//! The seam for hardware video encoders. The platform crate supplies one (NVIDIA's NVENC on
//! Windows and Linux, VideoToolbox on macOS); this crate only knows the shape of it, so the
//! recorder can be tested with a stand-in and falls back to the software encoder on any trouble.

use clipcraft_capture::NativeCaptureFrame;
use std::fmt;
use std::sync::Arc;

/// What the recorder asks a hardware encoder for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HardwareRequest {
    /// Picture width; even.
    pub width: u32,
    /// Picture height; even.
    pub height: u32,
    /// Frames per second.
    pub fps: u32,
    /// Target bitrate in kilobits per second.
    pub video_kbps: u32,
    /// Hold the bitrate steady.
    pub constant_bitrate: bool,
    /// Frames between keyframes.
    pub keyframe_frames: u32,
}

/// One encoded picture from a hardware encoder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HardwarePacket {
    /// Length-prefixed H.264 NAL units.
    pub data: Vec<u8>,
    /// True for a keyframe.
    pub keyframe: bool,
    /// True when the picture is shown at a different time than it is decoded (B-frames). The
    /// recorder cannot write those yet and treats them as a failure of the hardware encoder.
    pub reordered: bool,
}

/// A running hardware encoder. Pictures go in as straight 8-bit RGBA; the encoder converts them.
pub trait HardwareEncoder: Send {
    /// Encodes one picture. `index` is its time in frames.
    fn encode(&mut self, rgba: &[u8], index: u64) -> Result<Vec<HardwarePacket>, String>;

    /// Encodes a platform surface directly when this encoder can share its device and pixel
    /// format. `None` means the caller should read the surface back and use [`Self::encode`].
    ///
    /// The default keeps existing CPU-input encoders source-compatible. A platform implementation
    /// may downcast the opaque surface only inside `clipcraft-platform`.
    fn encode_surface(
        &mut self,
        _frame: &mut dyn NativeCaptureFrame,
        _index: u64,
    ) -> Result<Option<Vec<HardwarePacket>>, String> {
        Ok(None)
    }

    /// Finishes the stream.
    fn flush(&mut self) -> Result<Vec<HardwarePacket>, String>;

    /// The H.264 decoder configuration record (`avcC`). Only meaningful after the first
    /// picture has been encoded.
    fn avcc(&self) -> Vec<u8>;

    /// What to call this encoder, such as `NVIDIA NVENC`.
    fn name(&self) -> String;
}

type Open = dyn Fn(&HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String> + Send + Sync;

/// Opens hardware encoders. Cheap to clone.
#[derive(Clone)]
pub struct HardwareProvider {
    open: Arc<Open>,
}

impl HardwareProvider {
    /// Wraps a function that opens an encoder, or says why it cannot.
    pub fn new(
        open: impl Fn(&HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            open: Arc::new(open),
        }
    }

    /// Opens an encoder for `request`.
    pub fn open(&self, request: &HardwareRequest) -> Result<Box<dyn HardwareEncoder>, String> {
        (self.open)(request)
    }
}

impl fmt::Debug for HardwareProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HardwareProvider")
    }
}

impl PartialEq for HardwareProvider {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.open, &other.open)
    }
}
