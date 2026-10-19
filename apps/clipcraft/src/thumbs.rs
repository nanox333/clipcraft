//! Thumbnail textures for the library grid.
//!
//! Thumbnail JPEGs live in an on-disk cache. A texture is created only when a card is on screen,
//! a few per frame, and textures that stopped being visible are dropped once there are many, so a
//! library of thousands of clips never holds thousands of pictures in memory.

use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Textures kept before the ones not drawn last frame are dropped.
const MAX_TEXTURES: usize = 400;
/// Cache files decoded per frame, so scrolling a big library never stalls a frame.
const MAX_LOADS_PER_FRAME: usize = 6;

/// Which clips have a cached thumbnail, and the textures currently loaded for them.
#[derive(Default)]
pub struct Thumbnails {
    files: HashMap<String, PathBuf>,
    textures: HashMap<String, egui::TextureHandle>,
    seen: HashSet<String>,
    loaded_this_frame: usize,
}

impl Thumbnails {
    /// Records that `clip` has a cached thumbnail at `file`, replacing any older picture.
    pub fn set_file(&mut self, clip: String, file: PathBuf) {
        self.textures.remove(&clip);
        self.files.insert(clip, file);
    }

    /// Whether a thumbnail file is known for `clip`.
    #[cfg(test)]
    pub fn has_file(&self, clip: &str) -> bool {
        self.files.contains_key(clip)
    }

    /// Call at the start of every frame.
    pub fn begin_frame(&mut self) {
        self.loaded_this_frame = 0;
    }

    /// Call at the end of every frame: frees textures that were not drawn when there are many.
    pub fn end_frame(&mut self) {
        if self.textures.len() > MAX_TEXTURES {
            let seen = &self.seen;
            self.textures.retain(|clip, _| seen.contains(clip));
        }
        self.seen.clear();
    }

    /// The texture for `clip` when its card is `visible` and a thumbnail is known.
    pub fn texture(
        &mut self,
        context: &egui::Context,
        clip: &str,
        visible: bool,
    ) -> Option<&egui::TextureHandle> {
        if !visible {
            return None;
        }
        self.seen.insert(clip.to_owned());
        if !self.textures.contains_key(clip) {
            let file = self.files.get(clip)?.clone();
            if self.loaded_this_frame >= MAX_LOADS_PER_FRAME {
                context.request_repaint();
                return None;
            }
            self.loaded_this_frame += 1;
            match clipcraft_player::thumbnail::load(&file) {
                Ok(thumbnail) => {
                    let size = [thumbnail.width as usize, thumbnail.height as usize];
                    // `from_rgba_unmultiplied` asserts on a size mismatch; refuse instead.
                    if size[0].checked_mul(size[1]).and_then(|n| n.checked_mul(4))
                        != Some(thumbnail.rgba.len())
                    {
                        self.files.remove(clip);
                        return None;
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, &thumbnail.rgba);
                    let texture = context.load_texture(
                        format!("thumb:{clip}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(clip.to_owned(), texture);
                }
                Err(_) => {
                    // A damaged or removed cache file: forget it so it is not retried every frame.
                    self.files.remove(clip);
                    return None;
                }
            }
        }
        self.textures.get(clip)
    }

    #[cfg(test)]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }
}

/// The part of a texture shown when it is cropped to fill a `target` aspect ratio, centered.
pub fn cover_uv(texture: egui::Vec2, target: egui::Vec2) -> egui::Rect {
    let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if texture.x <= 0.0 || texture.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return full;
    }
    let (texture_ratio, target_ratio) = (texture.x / texture.y, target.x / target.y);
    if texture_ratio > target_ratio {
        let margin = (1.0 - target_ratio / texture_ratio) / 2.0;
        egui::Rect::from_min_max(egui::pos2(margin, 0.0), egui::pos2(1.0 - margin, 1.0))
    } else {
        let margin = (1.0 - texture_ratio / target_ratio) / 2.0;
        egui::Rect::from_min_max(egui::pos2(0.0, margin), egui::pos2(1.0, 1.0 - margin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_player::thumbnail::Thumbnail;

    fn write_thumbnail_file(directory: &std::path::Path, name: &str) -> PathBuf {
        // Reuse the player crate's own cache writer through a real encode/decode round trip.
        let file = directory.join(name);
        let pixels = Thumbnail {
            width: 8,
            height: 4,
            rgba: vec![200; 8 * 4 * 4],
        };
        let rgb: Vec<u8> = pixels
            .rgba
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode(&rgb, 8, 4, image::ExtendedColorType::Rgb8)
            .expect("encode");
        std::fs::write(&file, bytes).expect("write");
        file
    }

    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "clipcraft-thumbs-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    #[test]
    fn cover_crop_fills_the_target_without_stretching() {
        let wide = cover_uv(egui::vec2(200.0, 100.0), egui::vec2(100.0, 100.0));
        assert!((wide.min.x - 0.25).abs() < 1e-6 && (wide.max.x - 0.75).abs() < 1e-6);
        assert_eq!((wide.min.y, wide.max.y), (0.0, 1.0));
        let tall = cover_uv(egui::vec2(100.0, 200.0), egui::vec2(100.0, 100.0));
        assert!((tall.min.y - 0.25).abs() < 1e-6 && (tall.max.y - 0.75).abs() < 1e-6);
        let same = cover_uv(egui::vec2(160.0, 90.0), egui::vec2(16.0, 9.0));
        assert!(same.min.x.abs() < 1e-6 && same.min.y.abs() < 1e-6);
        // Degenerate sizes fall back to the whole texture.
        let degenerate = cover_uv(egui::Vec2::ZERO, egui::vec2(1.0, 1.0));
        assert_eq!((degenerate.min.x, degenerate.max.x), (0.0, 1.0));
    }

    #[test]
    fn textures_load_only_for_visible_cards_and_a_few_per_frame() {
        let context = egui::Context::default();
        let directory = scratch();
        let mut thumbs = Thumbnails::default();
        for number in 0..10 {
            let file = write_thumbnail_file(&directory, &format!("{number}.jpg"));
            thumbs.set_file(format!("clip-{number}"), file);
        }

        thumbs.begin_frame();
        assert!(
            thumbs.texture(&context, "clip-0", false).is_none(),
            "offscreen cards load nothing"
        );
        assert_eq!(thumbs.texture_count(), 0);

        let loaded = (0..10)
            .filter(|n| {
                thumbs
                    .texture(&context, &format!("clip-{n}"), true)
                    .is_some()
            })
            .count();
        assert_eq!(
            loaded, MAX_LOADS_PER_FRAME,
            "the per-frame cap limits decoding"
        );

        thumbs.begin_frame();
        let loaded = (0..10)
            .filter(|n| {
                thumbs
                    .texture(&context, &format!("clip-{n}"), true)
                    .is_some()
            })
            .count();
        assert_eq!(loaded, 10, "the rest arrive on the next frame");
        // Unknown clips have no texture.
        assert!(thumbs.texture(&context, "never-cached", true).is_none());
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn damaged_cache_files_are_forgotten_not_retried_every_frame() {
        let context = egui::Context::default();
        let directory = scratch();
        let broken = directory.join("broken.jpg");
        std::fs::write(&broken, b"not a jpeg").expect("write");
        let mut thumbs = Thumbnails::default();
        thumbs.set_file("clip".to_owned(), broken);
        assert!(thumbs.has_file("clip"));

        thumbs.begin_frame();
        assert!(thumbs.texture(&context, "clip", true).is_none());
        assert!(
            !thumbs.has_file("clip"),
            "a bad file is dropped so it is not decoded again"
        );
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn replacing_a_clips_thumbnail_reloads_its_texture_and_old_textures_are_freed() {
        let context = egui::Context::default();
        let directory = scratch();
        let mut thumbs = Thumbnails::default();
        for number in 0..(MAX_TEXTURES + 20) {
            let file = write_thumbnail_file(&directory, &format!("{number}.jpg"));
            thumbs.set_file(format!("clip-{number}"), file);
        }
        // Draw everything over several frames, then only the first ten.
        for _frame in 0..((MAX_TEXTURES + 20) / MAX_LOADS_PER_FRAME + 2) {
            thumbs.begin_frame();
            for number in 0..(MAX_TEXTURES + 20) {
                let _ = thumbs.texture(&context, &format!("clip-{number}"), true);
            }
            thumbs.end_frame();
        }
        assert!(thumbs.texture_count() > MAX_TEXTURES);

        thumbs.begin_frame();
        for number in 0..10 {
            let _ = thumbs.texture(&context, &format!("clip-{number}"), true);
        }
        thumbs.end_frame();
        assert_eq!(
            thumbs.texture_count(),
            10,
            "textures not drawn last frame are freed"
        );

        // A new thumbnail file for the same clip drops the stale texture.
        let fresh = write_thumbnail_file(&directory, "fresh.jpg");
        thumbs.set_file("clip-0".to_owned(), fresh);
        assert_eq!(thumbs.texture_count(), 9);
        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}
