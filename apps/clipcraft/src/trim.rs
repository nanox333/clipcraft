//! Trimming the open clip: choose a start and an end, save the part between them as a new MP4.
//! The cut copies the pictures and sound without re-encoding, runs on its own thread, shows its
//! progress, and can be cancelled. The original is never changed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

use clipcraft_engine::TrimmedClip;
use eframe::egui;
use egui::RichText;

use crate::kit::{self, DANGER, Icon, LIME, MUTED, TEXT};

/// The shortest cut that can be saved, in seconds.
const MIN_SECONDS: f64 = 0.1;

struct Job {
    receiver: mpsc::Receiver<Result<TrimmedClip, String>>,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
}

/// The trim controls and the export in flight.
#[derive(Default)]
pub struct TrimState {
    /// Whether the trim row is showing.
    pub open: bool,
    start: Option<f64>,
    end: Option<f64>,
    job: Option<Job>,
    /// The last result: text, and whether it is an error.
    message: Option<(String, bool)>,
    /// A file that was just saved, for the library to pick up.
    saved: Option<PathBuf>,
}

impl TrimState {
    /// The range to save, in seconds: the marks, or the whole clip where a mark is missing.
    pub fn range(&self, duration: f64) -> (f64, f64) {
        let end = self.end.unwrap_or(duration).clamp(0.0, duration.max(0.0));
        let start = self.start.unwrap_or(0.0).clamp(0.0, end);
        (start, end)
    }

    /// True when the range is long enough to save.
    pub fn valid(&self, duration: f64) -> bool {
        let (start, end) = self.range(duration);
        end - start >= MIN_SECONDS
    }

    /// Marks where the new clip starts. A start at or after the end clears the end.
    pub fn set_start(&mut self, seconds: f64, duration: f64) {
        let seconds = seconds.clamp(0.0, duration.max(0.0));
        self.start = Some(seconds);
        if self.end.is_some_and(|end| end - seconds < MIN_SECONDS) {
            self.end = None;
        }
    }

    /// Marks where the new clip ends. An end at or before the start clears the start.
    pub fn set_end(&mut self, seconds: f64, duration: f64) {
        let seconds = seconds.clamp(0.0, duration.max(0.0));
        self.end = Some(seconds);
        if self
            .start
            .is_some_and(|start| seconds - start < MIN_SECONDS)
        {
            self.start = None;
        }
    }

    /// Forgets the marks.
    pub fn clear(&mut self) {
        self.start = None;
        self.end = None;
    }

    /// The marked range for drawing on the scrub bar, when there is one.
    pub fn marked(&self, duration: f64) -> Option<(f64, f64)> {
        (self.open && (self.start.is_some() || self.end.is_some())).then(|| self.range(duration))
    }

    /// True while a cut is being written.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// A file saved since the last call.
    pub fn take_saved(&mut self) -> Option<PathBuf> {
        self.saved.take()
    }

    /// Starts saving the marked range of `input` on another thread.
    pub fn save(&mut self, input: &str, duration: f64, context: &egui::Context) {
        if self.job.is_some() || !self.valid(duration) {
            return;
        }
        let input = PathBuf::from(input);
        let Some(output) = clipcraft_engine::trimmed_path(&input) else {
            self.message = Some((
                "There is no free name for the trimmed clip.".to_owned(),
                true,
            ));
            return;
        };
        let (start, end) = self.range(duration);
        let (start_ms, end_ms) = (
            (start * 1000.0).round() as u64,
            (end * 1000.0).round() as u64,
        );
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let (thread_progress, thread_cancel) = (progress.clone(), cancel.clone());
        let wake = context.clone();
        self.message = None;
        std::thread::spawn(move || {
            let mut report = |fraction: f32| {
                thread_progress.store(fraction.to_bits(), Ordering::Relaxed);
                wake.request_repaint();
            };
            let result = clipcraft_engine::trim_clip(
                &input,
                &output,
                start_ms,
                end_ms,
                &mut report,
                &thread_cancel,
            )
            .map_err(|error| error.to_string());
            let _ = sender.send(result);
            wake.request_repaint();
        });
        self.job = Some(Job {
            receiver,
            progress,
            cancel,
        });
    }

    /// Collects the result of a finished cut.
    pub fn poll(&mut self) {
        let Some(job) = self.job.as_ref() else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("The cut stopped unexpectedly.".to_owned())
            }
        };
        self.job = None;
        match result {
            Ok(clip) => {
                let name = std::path::Path::new(&clip.path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.message = Some((format!("Saved {name}"), false));
                self.saved = Some(PathBuf::from(clip.path));
            }
            // Cancelling is the person's own choice, not a failure to report.
            Err(text) if text.contains("cancelled") => self.message = None,
            Err(text) => self.message = Some((text, true)),
        }
    }

    /// The trim row under the controls.
    pub fn show(&mut self, ui: &mut egui::Ui, position: f64, duration: f64, input: &str) {
        if !self.open {
            return;
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let idle = self.job.is_none();
            if kit::labelled_button(ui, Icon::StepForward, "Start here", false, idle)
                .on_hover_text("The new clip starts at the picture on screen (I)")
                .clicked()
            {
                self.set_start(position, duration);
            }
            if kit::labelled_button(ui, Icon::StepBack, "End here", false, idle)
                .on_hover_text("The new clip ends at the picture on screen (O)")
                .clicked()
            {
                self.set_end(position, duration);
            }
            let (start, end) = self.range(duration);
            ui.label(
                RichText::new(format!(
                    "{} \u{2192} {}  \u{00b7}  {:.1} s",
                    clock(start),
                    clock(end),
                    end - start
                ))
                .color(TEXT)
                .monospace()
                .size(13.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(job) = self.job.as_ref() {
                    if kit::labelled_button(ui, Icon::Stop, "Cancel", false, true).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    let fraction = f32::from_bits(job.progress.load(Ordering::Relaxed));
                    ui.add(
                        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                            .desired_width(140.0)
                            .fill(LIME),
                    );
                } else {
                    let valid = self.valid(duration);
                    if kit::labelled_button(ui, Icon::Import, "Save trimmed clip", true, valid)
                        .on_hover_text(
                            "Saves a new MP4 next to the original. Nothing is re-encoded.",
                        )
                        .clicked()
                    {
                        self.save(input, duration, ui.ctx());
                    }
                    if kit::labelled_button(ui, Icon::Library, "Reset", false, true).clicked() {
                        self.clear();
                    }
                }
            });
        });
        if let Some((text, error)) = self.message.as_ref() {
            ui.label(
                RichText::new(text)
                    .color(if *error { DANGER } else { MUTED })
                    .size(12.5),
            );
        } else if self.start.is_none() && self.end.is_none() {
            ui.label(
                RichText::new("Play to where the clip should begin, press Start here (I), then do the same for the end (O).")
                    .color(MUTED)
                    .size(12.5),
            );
        }
    }
}

/// `m:ss.s`, for marks that need to be exact to a tenth of a second.
fn clock(seconds: f64) -> String {
    let tenths = (seconds.max(0.0) * 10.0).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_are_shown_to_a_tenth_of_a_second() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(65.25), "1:05.3");
        assert_eq!(clock(-3.0), "0:00.0");
    }

    #[test]
    fn marks_make_a_range_and_clamp_to_the_clip() {
        let mut trim = TrimState::default();
        assert_eq!(trim.range(10.0), (0.0, 10.0));
        trim.set_start(2.0, 10.0);
        trim.set_end(7.5, 10.0);
        assert_eq!(trim.range(10.0), (2.0, 7.5));
        trim.set_end(99.0, 10.0);
        assert_eq!(trim.range(10.0), (2.0, 10.0));
        trim.set_start(-5.0, 10.0);
        assert_eq!(trim.range(10.0).0, 0.0);
    }

    #[test]
    fn crossing_marks_clear_the_other_one_so_the_range_stays_valid() {
        let mut trim = TrimState::default();
        trim.set_end(3.0, 10.0);
        trim.set_start(5.0, 10.0);
        assert_eq!(trim.range(10.0), (5.0, 10.0), "the end was cleared");
        trim.set_end(4.0, 10.0);
        assert_eq!(trim.range(10.0), (0.0, 4.0), "the start was cleared");
        assert!(trim.valid(10.0));
        trim.set_start(3.95, 10.0);
        assert_eq!(trim.range(10.0), (3.95, 10.0));
    }

    #[test]
    fn a_tiny_or_empty_range_cannot_be_saved() {
        let mut trim = TrimState::default();
        assert!(!trim.valid(0.0));
        assert!(!trim.valid(0.05));
        trim.set_start(1.0, 1.05);
        assert!(!trim.valid(1.05));
        assert!(trim.valid(5.0));
    }

    #[test]
    fn the_scrub_bar_only_shows_marks_while_the_row_is_open() {
        let mut trim = TrimState::default();
        trim.set_start(1.0, 5.0);
        assert_eq!(trim.marked(5.0), None);
        trim.open = true;
        assert_eq!(trim.marked(5.0), Some((1.0, 5.0)));
        trim.clear();
        assert_eq!(trim.marked(5.0), None);
    }
}
