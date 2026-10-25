//! Clip thumbnails: one small JPEG per clip in a per-user cache.
//!
//! A thumbnail is the picture about a tenth of the way into the clip, shrunk to
//! [`THUMBNAIL_WIDTH`] pixels wide. The cache file name is derived from the clip's path, size and
//! modified time, so an edited or replaced clip gets a fresh thumbnail and the old file is simply
//! left to the size limit. Cache files are written atomically and the cache is bounded; deleting
//! the whole folder only costs regenerating thumbnails.

use super::{PlayerError, clip_info, open_source};
use clipcraft_time::Tick;
use filmcraft_media::FrameRequest;
use std::fmt;
use std::fs;
use std::io::{Cursor, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Width of a stored thumbnail in pixels. Narrower pictures are kept at their own width.
pub const THUMBNAIL_WIDTH: u32 = 320;
/// Most thumbnails kept in the cache folder.
pub const MAX_CACHED_THUMBNAILS: usize = 5_000;
/// Largest thumbnail file that will be read back.
const MAX_THUMBNAIL_FILE_BYTES: u64 = 1024 * 1024;
/// Largest thumbnail side accepted when reading a cache file back.
const MAX_THUMBNAIL_SIDE: u32 = 1024;
const JPEG_QUALITY: u8 = 85;
/// Run a cache prune after this many newly written thumbnails.
const PRUNE_EVERY: u32 = 64;

/// A decoded thumbnail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thumbnail {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Opaque RGBA, 4 bytes per pixel.
    pub rgba: Vec<u8>,
}

/// A thumbnail stored in the cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedThumbnail {
    /// The JPEG file.
    pub file: PathBuf,
    /// True when this call created it, false when it was already cached.
    pub created: bool,
}

/// Why a thumbnail could not be produced or read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThumbnailError {
    /// The clip or cache file could not be read.
    Unreadable,
    /// ClipCraft cannot decode this clip's format yet.
    Unsupported,
    /// The decoder rejected the stream.
    DecodeFailed,
    /// The decoder failed unexpectedly.
    Crashed,
    /// The cache folder could not be written.
    CacheFailed,
}

impl From<PlayerError> for ThumbnailError {
    fn from(error: PlayerError) -> Self {
        match error {
            PlayerError::Unreadable => Self::Unreadable,
            PlayerError::Unsupported => Self::Unsupported,
            PlayerError::DecodeFailed | PlayerError::RenderFailed => Self::DecodeFailed,
            PlayerError::Crashed => Self::Crashed,
        }
    }
}

impl fmt::Display for ThumbnailError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unreadable => "The video or its thumbnail could not be read.",
            Self::Unsupported => "ClipCraft cannot decode this video format yet.",
            Self::DecodeFailed => "The video could not be decoded. The file may be damaged.",
            Self::Crashed => "Thumbnail generation stopped unexpectedly.",
            Self::CacheFailed => "The thumbnail cache could not be written.",
        })
    }
}

impl std::error::Error for ThumbnailError {}

/// Decodes a picture from the clip at `path` and shrinks it to thumbnail size.
pub fn generate(path: &Path) -> Result<Thumbnail, ThumbnailError> {
    catch_unwind(AssertUnwindSafe(|| {
        filmcraft_media::cancel::with_background(true, || generate_inner(path))
    }))
    .unwrap_or(Err(ThumbnailError::Crashed))
}

fn generate_inner(path: &Path) -> Result<Thumbnail, ThumbnailError> {
    let _permit = super::scheduling::background(&std::sync::atomic::AtomicBool::new(false))
        .map_err(|_| ThumbnailError::Crashed)?;
    let source = open_source(path)?;
    let info = clip_info(&source)?;
    let last_frame = (info.duration - info.frame_rate.frame_duration()).max(Tick::ZERO);
    let position = info.duration.mul_ratio(1, 10).clamp(Tick::ZERO, last_frame);
    let frame = source
        .video_frame(FrameRequest {
            time: position,
            scale: 1.0,
        })
        .map_err(|_| ThumbnailError::DecodeFailed)?;

    // Reduce in the decoder's own Y'CbCr format first so a 4K picture is never expanded to a full
    // size RGBA image just to be shrunk.
    let factor = usize::try_from(frame.width / THUMBNAIL_WIDTH).unwrap_or(1);
    let small = frame.box_decimated(factor);
    let frame = small.as_ref().unwrap_or(&frame);
    let rgba = frame.to_rgba8();
    resize(&rgba, frame.width, frame.height, THUMBNAIL_WIDTH)
}

/// Area-averages RGBA down to `target_width`, keeping the aspect ratio. Pictures already at or
/// below that width are returned unchanged. Rejects buffers whose size does not match.
fn resize(
    rgba: &[u8],
    width: u32,
    height: u32,
    target_width: u32,
) -> Result<Thumbnail, ThumbnailError> {
    let (w, h) = (width as usize, height as usize);
    let expected = w.checked_mul(h).and_then(|pixels| pixels.checked_mul(4));
    if w == 0 || h == 0 || expected != Some(rgba.len()) {
        return Err(ThumbnailError::DecodeFailed);
    }
    if width <= target_width {
        return Ok(Thumbnail {
            width,
            height,
            rgba: rgba.to_vec(),
        });
    }
    let dw = target_width as usize;
    let dh = (h * dw / w).max(1);
    let mut out = vec![0_u8; dw * dh * 4];
    for dy in 0..dh {
        let (y0, y1) = (dy * h / dh, ((dy + 1) * h / dh).max(dy * h / dh + 1).min(h));
        for dx in 0..dw {
            let (x0, x1) = (dx * w / dw, ((dx + 1) * w / dw).max(dx * w / dw + 1).min(w));
            let mut sum = [0_u32; 3];
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * w + x) * 4;
                    if let Some(pixel) = rgba.get(at..at + 3) {
                        sum[0] += u32::from(pixel[0]);
                        sum[1] += u32::from(pixel[1]);
                        sum[2] += u32::from(pixel[2]);
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            let at = (dy * dw + dx) * 4;
            if let Some(target) = out.get_mut(at..at + 4) {
                target[0] = (sum[0] / count) as u8;
                target[1] = (sum[1] / count) as u8;
                target[2] = (sum[2] / count) as u8;
                target[3] = 255;
            }
        }
    }
    Ok(Thumbnail {
        width: target_width,
        height: dh as u32,
        rgba: out,
    })
}

/// FNV-1a: a stable hash, unlike `DefaultHasher`, so cache names survive Rust upgrades.
fn fnv1a(parts: &[&[u8]]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for part in parts {
        for byte in *part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Cache file name for a clip with this path, size, and modified time.
pub fn cache_file_name(path: &Path, size_bytes: u64, modified_nanos: u128) -> String {
    let hash = fnv1a(&[
        path.to_string_lossy().as_bytes(),
        &size_bytes.to_le_bytes(),
        &modified_nanos.to_le_bytes(),
    ]);
    format!("{hash:016x}.jpg")
}

/// Makes sure a thumbnail for `path` exists in `cache_dir`, generating it when missing.
pub fn ensure_cached(path: &Path, cache_dir: &Path) -> Result<CachedThumbnail, ThumbnailError> {
    let metadata = fs::metadata(path).map_err(|_| ThumbnailError::Unreadable)?;
    if !metadata.is_file() {
        return Err(ThumbnailError::Unreadable);
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    let file = cache_dir.join(cache_file_name(path, metadata.len(), modified));

    if fs::metadata(&file).is_ok_and(|cached| cached.is_file() && cached.len() > 0) {
        return Ok(CachedThumbnail {
            file,
            created: false,
        });
    }

    let thumbnail = generate(path)?;
    write_jpeg(&file, &thumbnail)?;
    static CREATED: AtomicU32 = AtomicU32::new(0);
    if CREATED.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == PRUNE_EVERY - 1 {
        prune(cache_dir, MAX_CACHED_THUMBNAILS);
    }
    Ok(CachedThumbnail {
        file,
        created: true,
    })
}

fn write_jpeg(file: &Path, thumbnail: &Thumbnail) -> Result<(), ThumbnailError> {
    let rgb: Vec<u8> = thumbnail
        .rgba
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY)
        .encode(
            &rgb,
            thumbnail.width,
            thumbnail.height,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|_| ThumbnailError::CacheFailed)?;

    let directory = file.parent().ok_or(ThumbnailError::CacheFailed)?;
    fs::create_dir_all(directory).map_err(|_| ThumbnailError::CacheFailed)?;
    let mut temporary = file.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    let result = (|| {
        let mut out = fs::File::create(&temporary)?;
        out.write_all(&bytes)?;
        out.sync_all()?;
        fs::rename(&temporary, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(ThumbnailError::CacheFailed);
    }
    Ok(())
}

/// Reads a cached thumbnail back as RGBA. Oversized or malformed files are refused.
pub fn load(file: &Path) -> Result<Thumbnail, ThumbnailError> {
    let metadata = fs::metadata(file).map_err(|_| ThumbnailError::Unreadable)?;
    if !metadata.is_file() || metadata.len() > MAX_THUMBNAIL_FILE_BYTES {
        return Err(ThumbnailError::Unreadable);
    }
    let bytes = fs::read(file).map_err(|_| ThumbnailError::Unreadable)?;
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_THUMBNAIL_SIDE);
    limits.max_image_height = Some(MAX_THUMBNAIL_SIDE);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| ThumbnailError::DecodeFailed)?
        .to_rgba8();
    Ok(Thumbnail {
        width: decoded.width(),
        height: decoded.height(),
        rgba: decoded.into_raw(),
    })
}

/// Deletes the oldest `.jpg` files so at most `max_files` remain (and a little headroom is freed).
/// Only `.jpg` files in `cache_dir` are ever touched.
pub fn prune(cache_dir: &Path, max_files: usize) {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jpg"))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some((metadata.modified().ok()?, entry.path()))
        })
        .collect();
    if files.len() <= max_files {
        return;
    }
    files.sort();
    let keep = max_files - max_files / 10;
    let remove = files.len().saturating_sub(keep);
    for (_, path) in files.into_iter().take(remove) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_averages_blocks_and_keeps_aspect_ratio() {
        // 4x2 picture: left half black, right half white, shrunk to 2 wide.
        let mut rgba = Vec::new();
        for _row in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let small = resize(&rgba, 4, 2, 2).expect("resize");
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.rgba, [0, 0, 0, 255, 255, 255, 255, 255]);

        // Already small enough: unchanged.
        let same = resize(&rgba, 4, 2, 320).expect("resize");
        assert_eq!(
            (same.width, same.height, same.rgba.len()),
            (4, 2, rgba.len())
        );
    }

    #[test]
    fn resize_rejects_mismatched_buffers_and_handles_extreme_shapes() {
        assert_eq!(resize(&[0; 7], 2, 1, 1), Err(ThumbnailError::DecodeFailed));
        assert_eq!(resize(&[], 0, 0, 320), Err(ThumbnailError::DecodeFailed));
        assert_eq!(
            resize(&[], u32::MAX, u32::MAX, 320),
            Err(ThumbnailError::DecodeFailed)
        );
        // A one-pixel-tall strip must not produce a zero-height thumbnail.
        let strip = vec![128; 1000 * 4];
        let small = resize(&strip, 1000, 1, 320).expect("resize");
        assert_eq!((small.width, small.height), (320, 1));
    }

    #[test]
    fn cache_names_change_with_the_file_and_are_stable() {
        let path = Path::new("/clips/a.mp4");
        let name = cache_file_name(path, 100, 5);
        assert_eq!(name, cache_file_name(path, 100, 5));
        assert_ne!(name, cache_file_name(path, 101, 5));
        assert_ne!(name, cache_file_name(path, 100, 6));
        assert_ne!(name, cache_file_name(Path::new("/clips/b.mp4"), 100, 5));
        assert!(name.ends_with(".jpg") && name.len() == 20);
        // Pinned (and checked against an independent FNV-1a) so a hashing change is noticed:
        // existing caches would otherwise silently orphan.
        assert_eq!(name, "fcb3d17ed42942e4.jpg");
    }
}
