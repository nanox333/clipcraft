//! Still pictures of the screen being clipped, saved as PNG files next to the clips.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clipcraft_capture::CaptureFrame;
use clipcraft_record::Recorder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

use crate::EngineError;
use crate::session::clip_stem;

/// How long a screenshot waits for the next picture to arrive.
const WAIT: Duration = Duration::from_secs(2);

/// `ClipCraft Screenshot 2026-10-19 21-04-55` for a Unix time, in UTC.
pub fn screenshot_name(unix_seconds: u64) -> String {
    let stem = clip_stem(unix_seconds);
    match stem.strip_prefix("ClipCraft ") {
        Some(rest) => format!("ClipCraft Screenshot {rest}"),
        None => format!("{stem} screenshot"),
    }
}

/// A screenshot that can be taken on any thread.
pub struct ScreenshotJob {
    pub(crate) recorder: Arc<Recorder>,
    pub(crate) folder: PathBuf,
}

impl ScreenshotJob {
    /// Takes the next picture the recording captures and saves it, returning the file.
    pub fn run(self) -> Result<PathBuf, EngineError> {
        let frame = self
            .recorder
            .screenshot(WAIT)
            .ok_or(EngineError::ScreenshotFailed)?;
        fs::create_dir_all(&self.folder).map_err(|_| EngineError::ScreenshotFailed)?;
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let path = free_path(&self.folder, &screenshot_name(unix))?;
        write_png(&path, &frame)?;
        Ok(path)
    }
}

fn free_path(folder: &Path, stem: &str) -> Result<PathBuf, EngineError> {
    for attempt in 1..=999_u32 {
        let name = if attempt == 1 {
            format!("{stem}.png")
        } else {
            format!("{stem} ({attempt}).png")
        };
        let candidate = folder.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(EngineError::ScreenshotFailed)
}

/// Writes `frame` as a PNG, through a temporary file so a failure never leaves half a picture.
pub(crate) fn write_png(path: &Path, frame: &CaptureFrame) -> Result<(), EngineError> {
    frame
        .validate()
        .map_err(|_| EngineError::ScreenshotFailed)?;
    let width = frame.width as usize;
    let mut rgb = Vec::with_capacity(width * frame.height as usize * 3);
    for row in frame.bgra.chunks(frame.stride).take(frame.height as usize) {
        let pixels = row.get(..width * 4).ok_or(EngineError::ScreenshotFailed)?;
        for pixel in pixels.chunks_exact(4) {
            if let [blue, green, red, _] = pixel {
                rgb.extend_from_slice(&[*red, *green, *blue]);
            }
        }
    }
    let mut encoded = Vec::new();
    PngEncoder::new(&mut encoded)
        .write_image(&rgb, frame.width, frame.height, ExtendedColorType::Rgb8)
        .map_err(|_| EngineError::ScreenshotFailed)?;
    let partial = PathBuf::from(format!("{}.partial", path.to_string_lossy()));
    use std::io::Write;
    // The partial is claimed exclusively, and the picture never replaces an existing file.
    let Ok(mut file) = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
    else {
        return Err(EngineError::ScreenshotFailed);
    };
    let written = file.write_all(&encoded).and_then(|()| file.sync_all());
    drop(file);
    let published = written.is_ok()
        && match fs::hard_link(&partial, path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(_) => fs::symlink_metadata(path).is_err() && fs::rename(&partial, path).is_ok(),
        };
    let _ = fs::remove_file(&partial);
    if published {
        Ok(())
    } else {
        Err(EngineError::ScreenshotFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_capture::SyntheticSource;
    use clipcraft_time::Tick;

    #[test]
    fn names_are_utc_timestamps_next_to_the_clip_names() {
        assert_eq!(
            screenshot_name(951_782_400 + 3_661),
            "ClipCraft Screenshot 2000-02-29 01-01-01"
        );
    }

    #[test]
    fn a_picture_is_saved_as_a_png_with_its_colours_and_size() {
        let dir = std::env::temp_dir().join(format!("clipcraft-shot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // 3 x 2 pixels with a padded row: blue, green, red, white, then black and grey.
        let mut bgra = vec![0_u8; 16 * 2];
        let first = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
        let second = [[255, 255, 255, 255], [0, 0, 0, 255], [128, 128, 128, 255]];
        for (index, pixel) in first.iter().enumerate() {
            bgra[index * 4..index * 4 + 4].copy_from_slice(pixel);
        }
        for (index, pixel) in second.iter().enumerate() {
            bgra[16 + index * 4..16 + index * 4 + 4].copy_from_slice(pixel);
        }
        let frame = CaptureFrame {
            width: 3,
            height: 2,
            stride: 16,
            bgra,
            captured_at: Tick::from_units(0, 1),
        };
        let path = dir.join("shot.png");
        write_png(&path, &frame).unwrap();
        assert!(!dir.join("shot.png.partial").exists());
        let decoded = image::open(&path).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (3, 2));
        assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 255]);
        assert_eq!(decoded.get_pixel(1, 0).0, [0, 255, 0]);
        assert_eq!(decoded.get_pixel(2, 0).0, [255, 0, 0]);
        assert_eq!(decoded.get_pixel(2, 1).0, [128, 128, 128]);
        // A damaged frame is refused and leaves nothing behind.
        let damaged = CaptureFrame {
            bgra: vec![0; 4],
            ..frame
        };
        assert!(write_png(&dir.join("bad.png"), &damaged).is_err());
        assert!(!dir.join("bad.png").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_running_recording_gives_a_screenshot_and_a_stopped_one_does_not() {
        let dir = std::env::temp_dir().join(format!("clipcraft-shot-live-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut session = crate::CaptureSession::new();
        // Nothing is recording yet.
        assert!(session.screenshot_job(&dir).is_err());
        session
            .start(
                &crate::RecordingSettings::default(),
                Box::new(SyntheticSource::new(160, 90, 30)),
                None,
            )
            .unwrap();
        let path = session.screenshot_job(&dir).unwrap().run().unwrap();
        assert!(path.exists());
        assert!(path.extension().is_some_and(|extension| extension == "png"));
        let decoded = image::open(&path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (160, 90));
        // A second one in the same second gets its own name.
        let again = session.screenshot_job(&dir).unwrap().run().unwrap();
        assert_ne!(again, path);
        session.stop();
        let _ = fs::remove_dir_all(&dir);
    }
}
