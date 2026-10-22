//! Animated GIF export of a short range. GIF is large and limited to 256 colours per picture, so
//! the range, frame rate and width are bounded; frames are decoded, written and dropped one at a
//! time so memory use does not grow with the clip.

use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clipcraft_time::{FrameRate, Tick};
use filmcraft_media::reader::{FileReader, open_reader_within};
use filmcraft_media::{FrameRequest, SharedReader};

use crate::compress::{Temporary, check_cancel, source_size};
use crate::{ExportError, ExportReport, TrimRequest, io_error};

/// Longest range accepted, in seconds.
pub const MAX_GIF_SECONDS: i64 = 30;
/// Highest frame rate accepted.
pub const MAX_GIF_FPS: u32 = 20;

/// How a GIF is made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GifOptions {
    /// Widest picture, in pixels. The picture is never enlarged. 160 to 960.
    pub max_width: u32,
    /// Pictures per second, 5 to 20.
    pub fps: u32,
}

impl Default for GifOptions {
    fn default() -> Self {
        Self {
            max_width: 480,
            fps: 12,
        }
    }
}

impl GifOptions {
    fn check(self) -> Result<(), ExportError> {
        if (160..=960).contains(&self.max_width) && (5..=MAX_GIF_FPS).contains(&self.fps) {
            Ok(())
        } else {
            Err(ExportError::BadPreset)
        }
    }
}

/// Writes `request.start..request.end` of the input as a looping GIF at `request.output`.
///
/// The range must be from 0.1 to 30 seconds. The input is only read, an existing output is never
/// replaced, and a cancelled or failed export leaves no file behind.
pub fn export_gif(
    request: &TrimRequest,
    options: GifOptions,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    options.check()?;
    if request.start < Tick::ZERO || request.end <= request.start {
        return Err(ExportError::BadRange);
    }
    let duration = request.end - request.start;
    if duration < Tick::from_units(100, 1_000) {
        return Err(ExportError::BadRange);
    }
    if duration > Tick::from_units(MAX_GIF_SECONDS, 1) {
        return Err(ExportError::GifTooLong);
    }
    if fs::symlink_metadata(&request.output).is_ok() {
        return Err(ExportError::AlreadyExists);
    }
    check_cancel(cancel)?;
    catch_unwind(AssertUnwindSafe(|| {
        gif_inner(request, options, duration, progress, cancel)
    }))
    .unwrap_or(Err(ExportError::EncodeFailed))
}

fn gif_inner(
    request: &TrimRequest,
    options: GifOptions,
    duration: Tick,
    progress: &mut dyn FnMut(f32),
    cancel: &AtomicBool,
) -> Result<ExportReport, ExportError> {
    let reader: SharedReader =
        Arc::new(FileReader::open(&request.input).map_err(|_| ExportError::Unreadable)?);
    let name = request
        .input
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let decoder = open_reader_within(
        &name,
        reader,
        &filmcraft_codecs::reader_openers(),
        &filmcraft_codecs::openers(),
        64 * 1024 * 1024,
    )
    .map_err(|_| ExportError::Unreadable)?;
    let info = decoder.info().video.as_ref().ok_or(ExportError::Damaged)?;
    source_size(info.width, info.height)?;
    let (width, height) = gif_size(info.width, info.height, options.max_width);
    let rate = FrameRate {
        num: i64::from(options.fps),
        den: 1,
    };
    let frames =
        rate.frame_at(duration) + i64::from(rate.tick_of(rate.frame_at(duration)) < duration);
    if frames < 1 {
        return Err(ExportError::BadRange);
    }
    // GIF stores hundredths of a second. Round the cumulative clock, not each frame's
    // interval, so rates such as 12 or 13 fps do not accumulate playback-speed error.
    let centiseconds = (duration + Tick::from_units(1, 200)).to_units_floor(100);

    if let Some(folder) = request
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        fs::create_dir_all(folder).map_err(|e| io_error(&e))?;
    }
    let mut partial = request.output.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                ExportError::AlreadyExists
            } else {
                io_error(&e)
            }
        })?;
    let temporary = Temporary(partial);
    let w16 = u16::try_from(width).map_err(|_| ExportError::BadPreset)?;
    let h16 = u16::try_from(height).map_err(|_| ExportError::BadPreset)?;
    let mut encoder = gif::Encoder::new(BufWriter::new(file), w16, h16, &[])
        .map_err(|_| ExportError::WriteFailed)?;
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|_| ExportError::WriteFailed)?;
    // Pictures are independent, so a batch of them is decoded, resized and quantised on separate
    // threads and then written in order. Memory is bounded by the batch size.
    let workers = i64::try_from(crate::compress::worker_threads()).unwrap_or(2);
    let mut index = 0;
    while index < frames {
        check_cancel(cancel)?;
        let batch = (frames - index).min(workers);
        let decoder = &decoder;
        let made: Vec<Result<gif::Frame<'static>, ExportError>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (index..index + batch)
                .map(|n| {
                    scope.spawn(move || -> Result<gif::Frame<'static>, ExportError> {
                        let decoded = decoder
                            .video_frame(FrameRequest::full(request.start + rate.tick_of(n)))
                            .map_err(|_| ExportError::EncodeFailed)?;
                        source_size(decoded.width, decoded.height)?;
                        let factor =
                            (decoded.width / width).min(decoded.height / height).max(1) as usize;
                        let small = decoded.box_decimated(factor);
                        let picture = small.as_ref().unwrap_or(&decoded);
                        let image = image::RgbaImage::from_raw(
                            picture.width,
                            picture.height,
                            picture.to_rgba8(),
                        )
                        .ok_or(ExportError::Damaged)?;
                        let mut resized = if (picture.width, picture.height) == (width, height) {
                            image
                        } else {
                            crate::compress::resize_bilinear(&image, width, height)
                        };
                        let mut frame = gif::Frame::from_rgba_speed(w16, h16, resized.as_mut(), 10);
                        frame.delay = frame_delay(n, frames, centiseconds, options.fps)?;
                        Ok(frame)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap_or(Err(ExportError::EncodeFailed)))
                .collect()
        });
        for frame in made {
            encoder
                .write_frame(&frame?)
                .map_err(|_| ExportError::WriteFailed)?;
        }
        index += batch;
        progress((index as f32 / frames as f32 * 0.98).min(0.98));
    }
    check_cancel(cancel)?;
    let mut out = encoder.into_inner().map_err(|_| ExportError::WriteFailed)?;
    out.flush().map_err(|e| io_error(&e))?;
    out.get_ref().sync_all().map_err(|e| io_error(&e))?;
    let bytes = out.get_ref().metadata().map_err(|e| io_error(&e))?.len();
    drop(out);
    fs::hard_link(&temporary.0, &request.output).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => ExportError::AlreadyExists,
        std::io::ErrorKind::Unsupported => ExportError::Unsupported(
            "choose an output folder on a filesystem supporting hard links",
        ),
        _ => io_error(&e),
    })?;
    progress(1.0);
    Ok(ExportReport {
        path: request.output.clone(),
        duration,
        pictures: frames as u64,
        audio_tracks: 0,
        bytes,
    })
}

/// Quantize cumulative frame boundaries while preserving the rounded total duration. Reserve
/// at least two hundredths for a very short final frame: GIF readers commonly replace zero/one
/// hundredth delays with a much longer default. Earlier intervals lend it the needed time.
fn frame_delay(index: i64, frames: i64, centiseconds: i64, fps: u32) -> Result<u16, ExportError> {
    let boundary = |frame: i64| {
        let nominal = (frame * 100 + i64::from(fps) / 2) / i64::from(fps);
        nominal.min(centiseconds - (frames - frame) * 2)
    };
    u16::try_from(boundary(index + 1) - boundary(index)).map_err(|_| ExportError::BadRange)
}

/// Output size: at most `max_width` wide, never enlarged, aspect kept.
fn gif_size(width: u32, height: u32, max_width: u32) -> (u32, u32) {
    let w = width.min(max_width).max(2);
    let h = (u64::from(height) * u64::from(w) / u64::from(width.max(1))) as u32;
    (w, h.max(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_keeps_aspect_and_never_enlarges() {
        assert_eq!(gif_size(1920, 1080, 480), (480, 270));
        assert_eq!(gif_size(320, 180, 480), (320, 180));
        assert_eq!(gif_size(1080, 1920, 480), (480, 853));
    }

    #[test]
    fn options_are_bounded() {
        assert!(GifOptions::default().check().is_ok());
        assert!(
            GifOptions {
                max_width: 100,
                fps: 12
            }
            .check()
            .is_err()
        );
        assert!(
            GifOptions {
                max_width: 480,
                fps: 60
            }
            .check()
            .is_err()
        );
    }

    #[test]
    fn every_supported_frame_rate_keeps_the_cumulative_clock_and_short_final_frame() {
        for fps in 5..=MAX_GIF_FPS {
            let rate = FrameRate {
                num: i64::from(fps),
                den: 1,
            };
            for millis in [
                100, 101, 109, 501, 1_000, 1_001, 1_009, 1_237, 29_999, 30_000,
            ] {
                let duration = Tick::from_units(millis, 1_000);
                let frames = rate.frame_at(duration)
                    + i64::from(rate.tick_of(rate.frame_at(duration)) < duration);
                let total = (duration + Tick::from_units(1, 200)).to_units_floor(100);
                let delays: Vec<_> = (0..frames)
                    .map(|index| frame_delay(index, frames, total, fps).unwrap())
                    .collect();
                assert!(delays.iter().all(|delay| *delay >= 2));
                assert_eq!(
                    delays.iter().map(|delay| i64::from(*delay)).sum::<i64>(),
                    total
                );
                assert!((total * 10 - millis).abs() <= 5);
            }
        }
    }
}
