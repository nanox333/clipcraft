//! Immutable edit recipe consumed by the existing playback decoder worker.
use super::Compositor;
use crate::{ExportError, SegmentLook, SequenceText, TextRaster};
use clipcraft_time::Tick;
use image::RgbaImage;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewRecipe {
    pub canvas: Option<[u32; 2]>,
    pub source_start: Tick,
    pub source_end: Tick,
    pub timeline_start: Tick,
    pub look: SegmentLook,
    pub texts: Vec<SequenceText>,
}
impl Default for PreviewRecipe {
    fn default() -> Self {
        Self {
            canvas: None,
            source_start: Tick::ZERO,
            source_end: Tick::from_units(86400, 1),
            timeline_start: Tick::ZERO,
            look: SegmentLook::default(),
            texts: Vec::new(),
        }
    }
}
#[derive(Default)]
pub struct PreviewRenderer {
    compositor: Compositor,
    texts: HashMap<usize, TextRaster>,
}
impl PreviewRenderer {
    pub fn frame(
        &mut self,
        image: RgbaImage,
        source_time: Tick,
        recipe: &PreviewRecipe,
    ) -> Result<RgbaImage, ExportError> {
        let max = Tick::from_units(86400, 1);
        if source_time < Tick::ZERO
            || source_time > max
            || recipe.source_start < Tick::ZERO
            || recipe.source_end <= recipe.source_start
            || recipe.source_end > max
            || recipe.timeline_start < Tick::ZERO
            || recipe
                .timeline_start
                .0
                .checked_add((recipe.source_end - recipe.source_start).0)
                .is_none_or(|t| t > max.0)
            || recipe.texts.len() > crate::MAX_TEXT_LAYERS
            || recipe.texts.iter().any(|t| !t.is_valid())
        {
            return Err(ExportError::InvalidSequence);
        }
        let canvas = recipe.canvas.unwrap_or_else(|| {
            let limit = if image.width() >= image.height() {
                [3840, 2160]
            } else {
                [2160, 3840]
            };
            let fit = super::fit_size(image.dimensions(), limit);
            [fit[0].max(2), fit[1].max(2)]
        });
        let elapsed = (source_time - recipe.source_start)
            .clamp(Tick::ZERO, recipe.source_end - recipe.source_start);
        let time = recipe.timeline_start + elapsed;
        crate::sequence::compose_frame(
            image,
            canvas,
            recipe.look.effect,
            recipe
                .look
                .fade_at(elapsed, recipe.source_end - recipe.source_start),
            &recipe.texts,
            time,
            &mut self.texts,
            &mut self.compositor,
            &AtomicBool::new(false),
        )
    }
}
