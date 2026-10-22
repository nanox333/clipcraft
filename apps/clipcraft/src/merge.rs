//! Joining clips: queue clips from the library, then write them as one MP4 on another thread.
//! Nothing is re-encoded, so the clips must have been recorded with the same settings. The
//! originals are never changed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

use clipcraft_engine::{MAX_MERGE_CLIPS, TrimmedClip, format_bytes};
use eframe::egui;
use egui::RichText;

use crate::kit::{self, Icon, LIME, MUTED, TEXT};

struct Job {
    receiver: mpsc::Receiver<Result<TrimmedClip, String>>,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// What the merge bar asks the app to do.
#[derive(Debug, Eq, PartialEq)]
pub enum MergeEvent {
    /// A merged clip was written and the library should list it.
    Saved { message: String, path: PathBuf },
    /// The merge failed.
    Failed(String),
}

/// Clips waiting to be joined, and the join in flight.
#[derive(Default)]
pub struct MergeState {
    queue: Vec<String>,
    job: Option<Job>,
}

impl MergeState {
    /// The queued clips, in the order they will play.
    pub fn queue(&self) -> &[String] {
        &self.queue
    }

    /// Queues `path` at the end, or takes it out when it is already queued. A full queue ignores
    /// the clip. Returns whether the clip is queued afterwards.
    pub fn toggle(&mut self, path: &str) -> bool {
        if let Some(position) = self.queue.iter().position(|queued| queued == path) {
            self.queue.remove(position);
            return false;
        }
        if self.queue.len() >= MAX_MERGE_CLIPS {
            return false;
        }
        self.queue.push(path.to_owned());
        true
    }

    /// Starts joining the queued clips on another thread.
    pub fn start(&mut self, context: &egui::Context) -> Result<(), String> {
        if self.job.is_some() || self.queue.len() < 2 {
            return Ok(());
        }
        let inputs: Vec<PathBuf> = self.queue.iter().map(PathBuf::from).collect();
        let output = inputs
            .first()
            .and_then(|first| clipcraft_engine::merged_path(first))
            .ok_or_else(|| "There is no free name for the merged clip.".to_owned())?;
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let (thread_progress, thread_cancel) = (progress.clone(), cancel.clone());
        let wake = context.clone();
        std::thread::Builder::new()
            .name("clipcraft-merge".to_owned())
            .spawn(move || {
                let mut report = |fraction: f32| {
                    thread_progress.store(fraction.to_bits(), Ordering::Relaxed);
                    wake.request_repaint();
                };
                let result =
                    clipcraft_engine::merge_clips(&inputs, &output, &mut report, &thread_cancel)
                        .map_err(|error| error.to_string());
                let _ = sender.send(result);
                wake.request_repaint();
            })
            .map_err(|_| "The merge could not start. Try again.".to_owned())?;
        self.job = Some(Job {
            receiver,
            progress,
            cancel,
        });
        Ok(())
    }

    /// Collects a finished join. A success empties the queue; a failure keeps it so the person
    /// can change it and try again.
    pub fn poll(&mut self) -> Option<MergeEvent> {
        let job = self.job.as_ref()?;
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The merge stopped unexpectedly.".to_owned())
            }
        };
        self.job = None;
        match result {
            Ok(clip) => {
                self.queue.clear();
                let name = std::path::Path::new(&clip.path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Some(MergeEvent::Saved {
                    message: format!("Saved {name} \u{00b7} {}", format_bytes(clip.bytes)),
                    path: PathBuf::from(clip.path),
                })
            }
            // Cancelling is the person's own choice, not a failure to report.
            Err(text) if text.contains("cancelled") => None,
            Err(text) => Some(MergeEvent::Failed(text)),
        }
    }

    /// The bar above the library while clips are queued. Returns true when the person asked to
    /// start the join.
    pub fn show(&mut self, ui: &mut egui::Ui) -> bool {
        if self.queue.is_empty() && self.job.is_none() {
            return false;
        }
        let mut start = false;
        egui::Frame::new()
            .fill(kit::CARD)
            .stroke(egui::Stroke::new(1.0_f32, kit::BORDER))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.set_min_height(34.0);
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let count = self.queue.len();
                    let text = if count < 2 {
                        "1 clip queued \u{2014} add at least one more from a clip's menu".to_owned()
                    } else {
                        format!("{count} clips will be joined in the order you added them")
                    };
                    ui.label(RichText::new(text).color(TEXT).size(14.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(job) = self.job.as_ref() {
                            if kit::labelled_button(ui, Icon::Stop, "Cancel", false, true).clicked()
                            {
                                job.cancel.store(true, Ordering::Relaxed);
                            }
                            let fraction = f32::from_bits(job.progress.load(Ordering::Relaxed));
                            ui.add(
                                egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                                    .desired_width(140.0)
                                    .fill(LIME),
                            );
                        } else {
                            if kit::labelled_button(
                                ui,
                                Icon::Scissors,
                                "Join clips",
                                true,
                                self.queue.len() >= 2,
                            )
                            .on_hover_text(
                                "Saves one MP4 next to the first clip. Nothing is re-encoded.",
                            )
                            .clicked()
                            {
                                start = true;
                            }
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new("Clear").color(MUTED).size(13.0),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.queue.clear();
                            }
                        }
                    });
                });
            });
        ui.add_space(10.0);
        start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_queue_in_order_toggle_off_and_stop_at_the_limit() {
        let mut merge = MergeState::default();
        assert!(merge.toggle("a.mp4"));
        assert!(merge.toggle("b.mp4"));
        assert_eq!(merge.queue(), ["a.mp4", "b.mp4"]);
        assert!(merge.queue().contains(&"b.mp4".to_owned()));
        assert!(!merge.toggle("a.mp4"));
        assert_eq!(merge.queue(), ["b.mp4"]);
        merge = MergeState::default();
        for n in 0..MAX_MERGE_CLIPS {
            assert!(merge.toggle(&format!("{n}.mp4")));
        }
        assert!(!merge.toggle("one-too-many.mp4"));
        assert_eq!(merge.queue().len(), MAX_MERGE_CLIPS);
    }

    #[test]
    fn a_single_queued_clip_does_not_start_a_join() {
        let mut merge = MergeState::default();
        merge.toggle("a.mp4");
        assert!(merge.start(&egui::Context::default()).is_ok());
        assert!(merge.job.is_none());
    }

    #[test]
    fn the_bar_draws_empty_and_queued() {
        let context = egui::Context::default();
        kit::install_fonts(&context);
        for queued in [0, 1, 3] {
            let mut merge = MergeState::default();
            for n in 0..queued {
                merge.toggle(&format!("{n}.mp4"));
            }
            let _ = context.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!merge.show(ui));
                });
            });
        }
    }
}
