//! Recording preferences: output container, what is captured, and how it is framed.
//!
//! These are pure data and geometry. Nothing here touches the screen or a codec, so every rule is
//! unit-tested without a display. Capture itself is not implemented yet.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::EngineError;

/// Largest settings file ClipCraft will read.
pub const MAX_SETTINGS_BYTES: u64 = 64 * 1024;
/// Version of the settings file format.
pub const SETTINGS_VERSION: u32 = 1;
/// Largest output edge, in pixels.
pub const MAX_OUTPUT_EDGE: u32 = 7_680;
/// Smallest output edge, in pixels.
pub const MIN_OUTPUT_EDGE: u32 = 16;

/// The file format a clip is saved in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// MP4 with H.264 video and AAC audio.
    #[default]
    Mp4,
    /// Matroska with H.264 video and AAC audio.
    Mkv,
    /// WebM with VP9 video and Opus audio.
    Webm,
}

impl Container {
    /// Every container, in menu order.
    pub const ALL: [Container; 3] = [Container::Mp4, Container::Mkv, Container::Webm];

    /// The file extension, without a dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mkv => "MKV",
            Self::Webm => "WebM",
        }
    }

    /// A short description of what is inside the file.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Mp4 => "H.264 + AAC. Plays almost everywhere.",
            Self::Mkv => "H.264 + AAC. Survives a crash mid-recording.",
            Self::Webm => "VP9 + Opus. For the web.",
        }
    }

    /// Why this container cannot be written yet, or `None` when it can.
    pub fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::Mp4 | Self::Mkv => None,
            Self::Webm => Some("No VP9 or Opus encoder is available yet."),
        }
    }

    /// True when ClipCraft can write this container today.
    pub fn is_available(self) -> bool {
        self.unavailable_reason().is_none()
    }
}

/// The video compression format inside the file.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    /// H.264 (AVC), written by FilmCraft's pure-Rust encoder.
    #[default]
    H264,
    /// H.265 (HEVC): smaller files at the same quality.
    H265,
}

impl VideoCodec {
    /// Every codec, in menu order.
    pub const ALL: [VideoCodec; 2] = [VideoCodec::H264, VideoCodec::H265];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::H264 => "Works everywhere and encodes fast.",
            Self::H265 => "About a third smaller at the same quality.",
        }
    }

    /// Why this codec cannot be written yet, or `None` when it can.
    pub fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::H264 => None,
            Self::H265 => Some(
                "H.265 needs a hardware encoder (NVIDIA, Apple, or Intel/AMD); there is no software encoder yet.",
            ),
        }
    }

    /// True when ClipCraft can write this codec today.
    pub fn is_available(self) -> bool {
        self.unavailable_reason().is_none()
    }
}

/// What part of the machine is recorded.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    /// The window of the running game, including exclusive full-screen games.
    #[default]
    Game,
    /// A whole monitor, whatever is on it.
    Desktop,
}

impl CaptureSource {
    /// Every source, in menu order.
    pub const ALL: [CaptureSource; 2] = [CaptureSource::Game, CaptureSource::Desktop];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Game => "Game",
            Self::Desktop => "Desktop",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Game => "Records the game window, full screen or not.",
            Self::Desktop => "Records everything on one monitor.",
        }
    }
}

/// How the captured picture is fitted into the clip's frame.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Framing {
    /// Keep the source's shape. A 4:3 game stays 4:3.
    #[default]
    Original,
    /// Stretch to fill the output frame, distorting the picture (4:3 becomes wide 16:9).
    Stretch,
    /// Keep the shape and add black bars to fill the output frame.
    Letterbox,
    /// Keep the shape and cut off whatever overflows the output frame.
    Crop,
}

impl Framing {
    /// Every framing, in menu order.
    pub const ALL: [Framing; 4] = [
        Framing::Original,
        Framing::Stretch,
        Framing::Letterbox,
        Framing::Crop,
    ];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Stretch => "Stretch",
            Self::Letterbox => "Black bars",
            Self::Crop => "Crop",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Original => "Clip in the game's own shape.",
            Self::Stretch => "Fill the frame. A 4:3 game is stretched wide.",
            Self::Letterbox => "Keep the shape and fill the sides with black.",
            Self::Crop => "Keep the shape and trim the top and bottom.",
        }
    }
}

/// The shape of the clip's frame.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputShape {
    /// Same shape as the capture.
    #[default]
    Source,
    /// 16:9 widescreen.
    Wide16x9,
    /// 4:3 standard.
    Standard4x3,
}

impl OutputShape {
    /// Every shape, in menu order.
    pub const ALL: [OutputShape; 3] = [
        OutputShape::Source,
        OutputShape::Wide16x9,
        OutputShape::Standard4x3,
    ];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "Same as game",
            Self::Wide16x9 => "16:9",
            Self::Standard4x3 => "4:3",
        }
    }

    fn ratio(self) -> Option<(u32, u32)> {
        match self {
            Self::Source => None,
            Self::Wide16x9 => Some((16, 9)),
            Self::Standard4x3 => Some((4, 3)),
        }
    }
}

/// How the video encoder spends its bitrate.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RateControl {
    /// Spend more on busy scenes and less on calm ones, averaging the chosen bitrate.
    #[default]
    Variable,
    /// Hold the chosen bitrate steadily, which suits streaming and fixed-size limits.
    Constant,
}

impl RateControl {
    /// Every mode, in menu order.
    pub const ALL: [RateControl; 2] = [RateControl::Variable, RateControl::Constant];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Variable => "Variable",
            Self::Constant => "Constant",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Variable => "Best quality for the size. Bitrate follows the action.",
            Self::Constant => "Steady bitrate. Predictable file size.",
        }
    }
}

/// How hard the encoder works to shrink the video.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderPreset {
    /// Least CPU. Larger files at the same quality.
    Fast,
    /// A good balance of CPU and size.
    #[default]
    Balanced,
    /// Most CPU. Smallest files at the same quality.
    Quality,
}

impl EncoderPreset {
    /// Every preset, fastest first.
    pub const ALL: [EncoderPreset; 3] = [Self::Fast, Self::Balanced, Self::Quality];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Quality => "Quality",
        }
    }

    /// A short description.
    pub fn detail(self) -> &'static str {
        match self {
            Self::Fast => "Lowest CPU use, so the game keeps its frame rate. Larger files.",
            Self::Balanced => "A good balance of CPU use and file size.",
            Self::Quality => "Smallest files for the quality. Uses the most CPU.",
        }
    }
}

/// A named video quality: a bitrate that suits the clip's height and frame rate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualityPreset {
    /// Small files.
    Low,
    /// A good balance.
    Medium,
    /// Sharp, larger files.
    High,
    /// Near-lossless looking, very large files.
    Ultra,
}

impl QualityPreset {
    /// Every preset, lowest first.
    pub const ALL: [QualityPreset; 4] = [Self::Low, Self::Medium, Self::High, Self::Ultra];

    /// The name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Ultra => "Ultra",
        }
    }

    /// Bits per pixel per frame this preset aims for.
    fn bits_per_pixel(self) -> f64 {
        match self {
            Self::Low => 0.04,
            Self::Medium => 0.07,
            Self::High => 0.10,
            Self::Ultra => 0.16,
        }
    }

    /// The video bitrate, in kilobits per second, for a 16:9 clip of `height` at `frames_per_second`.
    pub fn video_kbps(self, height: u32, frames_per_second: u32) -> u32 {
        let height = u64::from(height.clamp(MIN_OUTPUT_EDGE, 4_320));
        let width = height * 16 / 9;
        let pixels_per_second =
            (width * height * u64::from(frames_per_second.clamp(1, 240))) as f64;
        let kbps = (pixels_per_second * self.bits_per_pixel() / 1000.0 / 100.0).round() * 100.0;
        (kbps as u32).clamp(MIN_VIDEO_KBPS, MAX_VIDEO_KBPS)
    }
}

/// Lowest accepted video bitrate.
pub const MIN_VIDEO_KBPS: u32 = 100;
/// Highest accepted video bitrate.
pub const MAX_VIDEO_KBPS: u32 = 200_000;

/// A rectangle of pixels inside a frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelRect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Where the source picture lands in the output frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reframe {
    /// Output frame width.
    pub output_width: u32,
    /// Output frame height.
    pub output_height: u32,
    /// The part of the source that is used.
    pub source: PixelRect,
    /// Where that part is drawn in the output. Pixels outside it are black.
    pub destination: PixelRect,
}

/// Everything the user chooses about recording.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct RecordingSettings {
    /// File format.
    pub container: Container,
    /// Video compression format.
    pub codec: VideoCodec,
    /// What is captured.
    pub source: CaptureSource,
    /// The shape of the clip's frame.
    pub shape: OutputShape,
    /// How the capture is fitted to that shape.
    pub framing: Framing,
    /// Clip height in pixels; the width follows from the shape.
    pub height: u32,
    /// Frames per second.
    pub frames_per_second: u32,
    /// Video bitrate in kilobits per second.
    pub video_kbps: u32,
    /// How the encoder spends that bitrate.
    pub rate_control: RateControl,
    /// Audio bitrate in kilobits per second.
    pub audio_kbps: u32,
    /// Whether the Advanced settings are shown.
    pub show_advanced: bool,
    /// How hard the encoder works.
    pub encoder_preset: EncoderPreset,
    /// Seconds between keyframes. Shorter is easier to scrub; longer is smaller.
    pub keyframe_seconds: u32,
    /// Audio sample rate in hertz.
    pub audio_sample_rate: u32,
    /// Audio channels: 1 (mono) or 2 (stereo).
    pub audio_channels: u32,
    /// Draw the mouse cursor in the clip.
    pub capture_cursor: bool,
    /// Record the game's and system's sound.
    pub capture_game_audio: bool,
    /// Record the microphone.
    pub capture_microphone: bool,
    /// Game sound level, 0 to 100 percent.
    pub game_audio_percent: u32,
    /// Microphone level, 0 to 100 percent.
    pub microphone_percent: u32,
    /// How many seconds of play before the hotkey are kept.
    pub replay_seconds: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            container: Container::Mp4,
            codec: VideoCodec::H264,
            source: CaptureSource::Game,
            shape: OutputShape::Source,
            framing: Framing::Original,
            height: 1080,
            frames_per_second: 60,
            video_kbps: 12_000,
            rate_control: RateControl::Variable,
            audio_kbps: 160,
            show_advanced: false,
            encoder_preset: EncoderPreset::Balanced,
            keyframe_seconds: 2,
            audio_sample_rate: 48_000,
            audio_channels: 2,
            capture_cursor: true,
            capture_game_audio: true,
            capture_microphone: false,
            game_audio_percent: 100,
            microphone_percent: 100,
            replay_seconds: 30,
        }
    }
}

/// Heights offered in the UI.
pub const HEIGHTS: [u32; 4] = [480, 720, 1080, 1440];
/// Frame rates offered in the UI.
pub const FRAME_RATES: [u32; 3] = [24, 30, 60];
/// Replay lengths offered in the UI.
pub const REPLAY_SECONDS: [u32; 5] = [15, 30, 60, 120, 300];
/// Video bitrates offered in the UI, in kilobits per second.
pub const VIDEO_KBPS: [u32; 8] = [2_500, 5_000, 8_000, 12_000, 16_000, 25_000, 40_000, 60_000];
/// Audio bitrates offered in the UI, in kilobits per second.
/// Keyframe intervals offered in the UI, in seconds.
pub const KEYFRAME_SECONDS: [u32; 4] = [1, 2, 4, 10];
/// Audio sample rates offered in the UI, in hertz.
pub const SAMPLE_RATES: [u32; 2] = [44_100, 48_000];
/// Audio bitrates offered in the UI, in kilobits per second.
pub const AUDIO_KBPS: [u32; 8] = [96, 128, 160, 192, 256, 320, 384, 512];

impl RecordingSettings {
    /// Clamps hand-edited values into safe ranges and swaps an unwritable container for MP4.
    pub fn sanitized(mut self) -> Self {
        if !self.container.is_available() {
            self.container = Container::Mp4;
        }
        if !self.codec.is_available() {
            self.codec = VideoCodec::H264;
        }
        self.height = self.height.clamp(MIN_OUTPUT_EDGE, 4_320);
        self.frames_per_second = self.frames_per_second.clamp(1, 240);
        self.video_kbps = self.video_kbps.clamp(MIN_VIDEO_KBPS, MAX_VIDEO_KBPS);
        self.audio_kbps = self.audio_kbps.clamp(32, 512);
        self.keyframe_seconds = self.keyframe_seconds.clamp(1, 30);
        if !SAMPLE_RATES.contains(&self.audio_sample_rate) {
            self.audio_sample_rate = 48_000;
        }
        self.audio_channels = self.audio_channels.clamp(1, 2);
        self.game_audio_percent = self.game_audio_percent.min(100);
        self.microphone_percent = self.microphone_percent.min(100);
        self.replay_seconds = self.replay_seconds.clamp(5, 600);
        // Distorting or trimming means nothing when the shape is the source's own.
        if self.shape == OutputShape::Source {
            self.framing = Framing::Original;
        } else if self.framing == Framing::Original {
            self.framing = Framing::Stretch;
        }
        self
    }

    /// Where a source of `width` x `height` lands in the output, or `None` for an empty source.
    pub fn reframe(&self, width: u32, height: u32) -> Option<Reframe> {
        reframe(
            width,
            height,
            self.shape,
            self.framing,
            self.height.clamp(MIN_OUTPUT_EDGE, 4_320),
        )
    }
}

fn even(value: u32) -> u32 {
    (value & !1).max(2)
}

/// Computes how a `width` x `height` capture is fitted into the output frame.
///
/// Output sizes are even, because H.264 4:2:0 needs even dimensions.
pub fn reframe(
    width: u32,
    height: u32,
    shape: OutputShape,
    framing: Framing,
    output_height: u32,
) -> Option<Reframe> {
    if width == 0 || height == 0 || width > 65_535 || height > 65_535 {
        return None;
    }
    let output_height = even(output_height.min(MAX_OUTPUT_EDGE));
    let whole = PixelRect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let (output_width, framing) = match shape.ratio() {
        None => {
            let scaled = u64::from(output_height) * u64::from(width) / u64::from(height);
            (
                even(scaled.min(u64::from(MAX_OUTPUT_EDGE)) as u32),
                Framing::Original,
            )
        }
        Some((numerator, denominator)) => {
            let scaled = u64::from(output_height) * u64::from(numerator) / u64::from(denominator);
            (even(scaled.min(u64::from(MAX_OUTPUT_EDGE)) as u32), framing)
        }
    };
    let full = PixelRect {
        x: 0,
        y: 0,
        width: output_width,
        height: output_height,
    };
    let (source, destination) = match framing {
        Framing::Original | Framing::Stretch => (whole, full),
        Framing::Letterbox => {
            // Fit inside: compare width/height ratios by cross-multiplying.
            let wide_source = u64::from(width) * u64::from(output_height)
                >= u64::from(height) * u64::from(output_width);
            let fitted = if wide_source {
                let fitted_height = u64::from(output_width) * u64::from(height) / u64::from(width);
                let fitted_height = even(fitted_height as u32).min(output_height);
                PixelRect {
                    x: 0,
                    y: ((output_height - fitted_height) / 2) & !1,
                    width: output_width,
                    height: fitted_height,
                }
            } else {
                let fitted_width = u64::from(output_height) * u64::from(width) / u64::from(height);
                let fitted_width = even(fitted_width as u32).min(output_width);
                PixelRect {
                    x: ((output_width - fitted_width) / 2) & !1,
                    y: 0,
                    width: fitted_width,
                    height: output_height,
                }
            };
            (whole, fitted)
        }
        Framing::Crop => {
            // Fill the output, cutting the source on the axis that overflows.
            let wide_source = u64::from(width) * u64::from(output_height)
                >= u64::from(height) * u64::from(output_width);
            let cut = if wide_source {
                let used_width = (u64::from(height) * u64::from(output_width)
                    / u64::from(output_height))
                .clamp(1, u64::from(width)) as u32;
                PixelRect {
                    x: (width - used_width) / 2,
                    y: 0,
                    width: used_width,
                    height,
                }
            } else {
                let used_height = (u64::from(width) * u64::from(output_height)
                    / u64::from(output_width))
                .clamp(1, u64::from(height)) as u32;
                PixelRect {
                    x: 0,
                    y: (height - used_height) / 2,
                    width,
                    height: used_height,
                }
            };
            (cut, full)
        }
    };
    Some(Reframe {
        output_width,
        output_height,
        source,
        destination,
    })
}

#[derive(Deserialize, Serialize)]
struct SettingsFile {
    version: u32,
    settings: RecordingSettings,
}

/// Default per-user settings file, or `None` when no home directory is known.
pub fn default_settings_path() -> Option<PathBuf> {
    Some(crate::index::default_index_path()?.with_file_name("recording-settings.json"))
}

/// Reads settings. A missing file yields the defaults; a damaged one is an error.
pub(crate) fn load_settings(path: &Path) -> Result<RecordingSettings, EngineError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecordingSettings::default());
        }
        Err(_) => return Err(EngineError::SettingsUnavailable),
    };
    if metadata.len() > MAX_SETTINGS_BYTES {
        return Err(EngineError::SettingsInvalid);
    }
    let bytes = fs::read(path).map_err(|_| EngineError::SettingsUnavailable)?;
    let file: SettingsFile =
        serde_json::from_slice(&bytes).map_err(|_| EngineError::SettingsInvalid)?;
    if file.version != SETTINGS_VERSION {
        return Err(EngineError::SettingsInvalid);
    }
    Ok(file.settings.sanitized())
}

/// Saves settings with a temp file and rename; returns what was stored.
pub(crate) fn save_settings(
    path: &Path,
    settings: RecordingSettings,
) -> Result<RecordingSettings, EngineError> {
    let settings = settings.sanitized();
    let bytes = serde_json::to_vec_pretty(&SettingsFile {
        version: SETTINGS_VERSION,
        settings: settings.clone(),
    })
    .map_err(|_| EngineError::SettingsSaveFailed)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|_| EngineError::SettingsSaveFailed)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(EngineError::SettingsSaveFailed);
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn source_shape_keeps_a_4_3_game_4_3() {
        let result = reframe(1600, 1200, OutputShape::Source, Framing::Stretch, 1080).unwrap();
        assert_eq!((result.output_width, result.output_height), (1440, 1080));
        assert_eq!(result.source, rect(0, 0, 1600, 1200));
        assert_eq!(result.destination, rect(0, 0, 1440, 1080));
    }

    #[test]
    fn stretching_4_3_to_16_9_fills_the_frame() {
        let result = reframe(1024, 768, OutputShape::Wide16x9, Framing::Stretch, 1080).unwrap();
        assert_eq!((result.output_width, result.output_height), (1920, 1080));
        assert_eq!(result.source, rect(0, 0, 1024, 768));
        assert_eq!(result.destination, rect(0, 0, 1920, 1080));
    }

    #[test]
    fn black_bars_center_a_4_3_game_in_16_9() {
        let result = reframe(1024, 768, OutputShape::Wide16x9, Framing::Letterbox, 1080).unwrap();
        assert_eq!(result.destination, rect(240, 0, 1440, 1080));
    }

    #[test]
    fn black_bars_center_a_wide_game_in_4_3() {
        let result = reframe(
            1920,
            1080,
            OutputShape::Standard4x3,
            Framing::Letterbox,
            1080,
        )
        .unwrap();
        assert_eq!((result.output_width, result.output_height), (1440, 1080));
        assert_eq!(result.destination, rect(0, 134, 1440, 810));
    }

    #[test]
    fn cropping_a_4_3_game_to_16_9_trims_top_and_bottom() {
        let result = reframe(1024, 768, OutputShape::Wide16x9, Framing::Crop, 1080).unwrap();
        assert_eq!(result.source, rect(0, 96, 1024, 576));
        assert_eq!(result.destination, rect(0, 0, 1920, 1080));
    }

    #[test]
    fn cropping_a_wide_game_to_4_3_trims_the_sides() {
        let result = reframe(1920, 1080, OutputShape::Standard4x3, Framing::Crop, 1080).unwrap();
        assert_eq!(result.source, rect(240, 0, 1440, 1080));
    }

    #[test]
    fn every_output_dimension_is_even_and_inside_the_frame() {
        for (width, height) in [(1, 1), (3, 7), (641, 479), (65_535, 2), (2, 65_535)] {
            for shape in OutputShape::ALL {
                for framing in Framing::ALL {
                    for out in [2, 17, 480, 1081, 100_000] {
                        let r = reframe(width, height, shape, framing, out).unwrap();
                        assert_eq!(r.output_width % 2, 0);
                        assert_eq!(r.output_height % 2, 0);
                        assert!(r.destination.x + r.destination.width <= r.output_width);
                        assert!(r.destination.y + r.destination.height <= r.output_height);
                        assert!(r.source.x + r.source.width <= width);
                        assert!(r.source.y + r.source.height <= height);
                        assert!(r.source.width > 0 && r.source.height > 0);
                        assert!(r.destination.width > 0 && r.destination.height > 0);
                    }
                }
            }
        }
    }

    #[test]
    fn an_empty_or_absurd_source_has_no_reframe() {
        assert!(reframe(0, 10, OutputShape::Source, Framing::Original, 1080).is_none());
        assert!(reframe(10, 0, OutputShape::Source, Framing::Original, 1080).is_none());
        assert!(reframe(70_000, 10, OutputShape::Source, Framing::Original, 1080).is_none());
    }

    #[test]
    fn presets_scale_with_resolution_and_frame_rate() {
        assert_eq!(QualityPreset::Medium.video_kbps(1080, 60), 8_700);
        let mut last = 0;
        for preset in QualityPreset::ALL {
            let kbps = preset.video_kbps(1080, 60);
            assert!(kbps > last);
            last = kbps;
        }
        assert!(QualityPreset::High.video_kbps(1080, 60) > QualityPreset::High.video_kbps(720, 60));
        assert!(
            QualityPreset::High.video_kbps(1080, 60) > QualityPreset::High.video_kbps(1080, 30)
        );
        for height in [0, 1, 100_000] {
            let kbps = QualityPreset::Ultra.video_kbps(height, 1_000);
            assert!((MIN_VIDEO_KBPS..=MAX_VIDEO_KBPS).contains(&kbps));
        }
    }

    #[test]
    fn every_audio_bitrate_fits_the_aac_stereo_limit() {
        // AAC allows 6144 bits per channel per 1024-sample frame: 576 kbps for stereo at 48 kHz,
        // 529 kbps at 44.1 kHz.
        for kbps in AUDIO_KBPS {
            assert!(u64::from(kbps) * 1000 * 1024 / 44_100 <= 6144 * 2);
            assert_eq!(
                RecordingSettings {
                    audio_kbps: kbps,
                    ..Default::default()
                }
                .sanitized()
                .audio_kbps,
                kbps
            );
        }
    }

    #[test]
    fn advanced_values_are_clamped_and_old_files_keep_working() {
        let settings = RecordingSettings {
            keyframe_seconds: 0,
            audio_sample_rate: 12_345,
            audio_channels: 9,
            game_audio_percent: 500,
            microphone_percent: 101,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(settings.keyframe_seconds, 1);
        assert_eq!(settings.audio_sample_rate, 48_000);
        assert_eq!(settings.audio_channels, 2);
        assert_eq!(settings.game_audio_percent, 100);
        assert_eq!(settings.microphone_percent, 100);
        // A file written before these options existed still loads with their defaults.
        let old: SettingsFile =
            serde_json::from_str(r#"{"version":1,"settings":{"height":720}}"#).unwrap();
        assert!(old.settings.capture_cursor && !old.settings.show_advanced);
    }

    #[test]
    fn h265_is_unavailable_and_falls_back_to_h264() {
        assert!(VideoCodec::H264.is_available());
        let settings = RecordingSettings {
            codec: VideoCodec::H265,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(settings.codec, VideoCodec::H264);
    }

    #[test]
    fn webm_is_unavailable_and_falls_back_to_mp4() {
        assert!(Container::Mp4.is_available() && Container::Mkv.is_available());
        assert!(Container::Webm.unavailable_reason().is_some());
        let settings = RecordingSettings {
            container: Container::Webm,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(settings.container, Container::Mp4);
    }

    #[test]
    fn sanitizing_clamps_values_and_reconciles_framing() {
        let settings = RecordingSettings {
            height: 0,
            frames_per_second: 9_999,
            video_kbps: 0,
            replay_seconds: 0,
            shape: OutputShape::Source,
            framing: Framing::Crop,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(settings.height, MIN_OUTPUT_EDGE);
        assert_eq!(settings.frames_per_second, 240);
        assert_eq!(settings.video_kbps, 100);
        assert_eq!(settings.replay_seconds, 5);
        assert_eq!(settings.framing, Framing::Original);
        let wide = RecordingSettings {
            shape: OutputShape::Wide16x9,
            framing: Framing::Original,
            ..RecordingSettings::default()
        }
        .sanitized();
        assert_eq!(wide.framing, Framing::Stretch);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("clipcraft-settings-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn settings_round_trip_and_missing_file_gives_defaults() {
        let dir = scratch("round");
        let path = dir.join("nested/settings.json");
        assert_eq!(load_settings(&path).unwrap(), RecordingSettings::default());
        let chosen = RecordingSettings {
            container: Container::Mkv,
            source: CaptureSource::Desktop,
            shape: OutputShape::Wide16x9,
            framing: Framing::Letterbox,
            ..RecordingSettings::default()
        };
        let saved = save_settings(&path, chosen.clone()).unwrap();
        assert_eq!(saved, chosen);
        assert_eq!(load_settings(&path).unwrap(), chosen);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn damaged_oversized_and_future_files_are_rejected() {
        let dir = scratch("bad");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        fs::write(&path, b"{not json").unwrap();
        assert_eq!(load_settings(&path), Err(EngineError::SettingsInvalid));
        fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES as usize + 1]).unwrap();
        assert_eq!(load_settings(&path), Err(EngineError::SettingsInvalid));
        fs::write(&path, br#"{"version":99,"settings":{}}"#).unwrap();
        assert_eq!(load_settings(&path), Err(EngineError::SettingsInvalid));
        // Unknown or missing fields fall back to defaults rather than losing the file.
        fs::write(&path, br#"{"version":1,"settings":{"height":720}}"#).unwrap();
        assert_eq!(load_settings(&path).unwrap().height, 720);
        let _ = fs::remove_dir_all(&dir);
    }
}
