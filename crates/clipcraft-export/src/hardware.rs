//! The seam for a faster video encoder, such as a graphics card's. The export crate only knows
//! the shape of one; the application supplies the real thing (see `clipcraft-platform`), and any
//! trouble with it makes the export fall back to the software encoder.

use std::sync::Arc;

/// What the export asks of a hardware encoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FastRequest {
    /// Picture width; even.
    pub width: u32,
    /// Picture height; even.
    pub height: u32,
    /// Pictures per second.
    pub fps: u32,
    /// Target video bitrate, in kilobits per second.
    pub video_kbps: u32,
    /// Pictures between keyframes.
    pub keyframe_frames: u32,
}

/// One encoded picture: length-prefixed H.264 NAL units, in the order the pictures went in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FastPacket {
    /// The compressed picture.
    pub data: Vec<u8>,
    /// Whether it is a keyframe.
    pub keyframe: bool,
}

/// A running encoder. Pictures go in as straight 8-bit RGBA, one packet comes out per picture
/// (possibly later), and nothing may be reordered.
pub trait FastEncoder: Send {
    /// Encodes the picture numbered `index`.
    fn encode(&mut self, rgba: &[u8], index: u64) -> Result<Vec<FastPacket>, String>;
    /// Finishes the stream and returns what is left.
    fn flush(&mut self) -> Result<Vec<FastPacket>, String>;
    /// The `avcC` record describing the stream; empty until the first picture is encoded.
    fn avcc(&self) -> Vec<u8>;
    /// What to call this encoder, such as `NVIDIA NVENC`.
    fn name(&self) -> String;
}

/// Opens an encoder, or says why it cannot. Cheap to clone and share between threads.
pub type FastOpener =
    Arc<dyn Fn(&FastRequest) -> Result<Box<dyn FastEncoder>, String> + Send + Sync>;
