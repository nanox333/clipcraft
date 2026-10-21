//! Records a screen or window into a bounded replay buffer and saves the last stretch of it as a
//! clip.
//!
//! A [`Recorder`] polls a [`clipcraft_capture::CaptureSource`] at the chosen frame rate, fits each
//! picture into the clip's frame ([`convert`]), encodes it with FilmCraft's pure-Rust H.264
//! encoder, and keeps the newest seconds in a [`ReplayBuffer`]. [`Recorder::save_clip`] writes that
//! buffer to an MP4 or MKV file. Everything is bounded: the picture queue, the buffer's seconds and
//! bytes, and the time spent waiting. Pictures the encoder cannot keep up with are dropped and
//! counted, never queued without limit.
//!
//! Sound comes from [`AudioSource`]s (microphone, system output), is mixed against the same clock
//! the pictures use, encoded as AAC, and buffered next to the video.

pub mod audio;
pub mod audio_timeline;
pub mod convert;
pub mod hardware;
pub mod mux;
mod recorder;
pub mod replay;

use std::fmt;

pub use audio::{
    AudioConfig, AudioControl, AudioError, AudioFormat, AudioInput, AudioInputStatus, AudioKind,
    AudioProvider, AudioSetup, AudioSource, SampleQueue, ToneSource,
};
pub use hardware::{HardwareEncoder, HardwarePacket, HardwareProvider, HardwareRequest};
pub use mux::{ClipAudio, ClipFormat, SavedClip, VideoInfo, write_clip, write_clip_interrupted};
pub use recorder::{EncoderSpeed, RecordConfig, RecordState, RecordStatus, Recorder, output_size};

/// Why recording or saving failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordError {
    /// A picture or output size is zero, odd, or out of range.
    BadSize,
    /// The capture provider returned a damaged picture.
    BadFrame,
    /// The encoder could not be created or refused a picture.
    EncoderFailed,
    /// Capture stopped; the buffered video can still be saved.
    Capture(clipcraft_capture::CaptureError),
    /// The buffer holds no video yet.
    NothingBuffered,
    /// The buffered stream cannot be described in a file.
    BadStream,
    /// A file with that name already exists; ClipCraft never overwrites clips.
    AlreadyExists,
    /// The disk is full.
    DiskFull,
    /// The folder cannot be written to.
    FolderNotWritable,
    /// The file could not be written.
    WriteFailed,
    /// The settings cannot be recorded as asked.
    BadConfig,
    /// A recording thread could not be started.
    ThreadFailed,
    /// The sound could not be set up.
    AudioFailed,
}

impl fmt::Display for RecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSize => formatter.write_str("The picture size cannot be recorded."),
            Self::BadFrame => formatter.write_str("The screen capture returned a damaged picture."),
            Self::EncoderFailed => formatter.write_str("The video encoder could not start."),
            Self::Capture(error) => error.fmt(formatter),
            Self::NothingBuffered => formatter
                .write_str("There is no video to save yet. Wait a moment after recording starts."),
            Self::BadStream => {
                formatter.write_str("The recorded video cannot be written to a file.")
            }
            Self::AlreadyExists => formatter.write_str("A clip with that name already exists."),
            Self::DiskFull => {
                formatter.write_str("The disk is full. Free some space and save again.")
            }
            Self::FolderNotWritable => {
                formatter.write_str("ClipCraft cannot write to the capture folder.")
            }
            Self::WriteFailed => formatter.write_str("The clip could not be written."),
            Self::BadConfig => formatter.write_str("These recording settings cannot be used."),
            Self::ThreadFailed => formatter.write_str("Recording could not be started."),
            Self::AudioFailed => formatter.write_str("The sound could not be set up."),
        }
    }
}

impl std::error::Error for RecordError {}
