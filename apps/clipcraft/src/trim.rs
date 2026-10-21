//! Trimming the open clip: choose a start and an end, save the part between them as a new MP4.
//! The cut copies the pictures and sound without re-encoding, runs on its own thread, shows its
//! progress, and can be cancelled. The original is never changed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

use clipcraft_engine::{ExportPreset, ExportRange, TrimmedClip, format_bytes};
use eframe::egui;
use egui::{Color32, RichText};

use crate::kit::{self, DANGER, Icon, LIME, MUTED, TEXT};

/// The shortest cut that can be saved, in seconds.
const MIN_SECONDS: f64 = 0.1;

struct Job {
    receiver: mpsc::Receiver<Result<Finished, String>>,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
}

struct Finished {
    clip: TrimmedClip,
    target: Option<u64>,
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
    /// The size of the clip being trimmed, read once.
    size_of: Option<(String, Option<u64>)>,
    /// None preserves the existing lossless trim workflow.
    preset: Option<ExportPreset>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
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
        let preset = self.preset;
        let output = if preset.is_some() {
            clipcraft_engine::exported_path(&input)
        } else {
            clipcraft_engine::trimmed_path(&input)
        };
        let Some(output) = output else {
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
        let spawned = std::thread::Builder::new()
            .name("clipcraft-export".to_owned())
            .spawn(move || {
                let mut report = |fraction: f32| {
                    thread_progress.store(fraction.to_bits(), Ordering::Relaxed);
                    wake.request_repaint();
                };
                let result = if let Some(preset) = preset {
                    let request = ExportRange {
                        input,
                        output,
                        start: clipcraft_time::Tick::from_units(
                            i64::try_from(start_ms).unwrap_or(i64::MAX),
                            1_000,
                        ),
                        end: clipcraft_time::Tick::from_units(
                            i64::try_from(end_ms).unwrap_or(i64::MAX),
                            1_000,
                        ),
                    };
                    clipcraft_engine::export_clip(&request, preset, &mut report, &thread_cancel)
                        .map(|report| Finished {
                            clip: report.clip,
                            target: Some(report.target_bytes),
                        })
                } else {
                    clipcraft_engine::trim_clip(
                        &input,
                        &output,
                        start_ms,
                        end_ms,
                        &mut report,
                        &thread_cancel,
                    )
                    .map(|clip| Finished { clip, target: None })
                }
                .map_err(|error| error.to_string());
                let _ = sender.send(result);
                wake.request_repaint();
            });
        if spawned.is_err() {
            self.message = Some(("The export could not start. Try again.".to_owned(), true));
            return;
        }
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
            Ok(Finished { clip, target }) => {
                let name = std::path::Path::new(&clip.path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let size = format_bytes(clip.bytes);
                let detail = target.map(|target| {
                    let target_size = format_bytes(target);
                    if clip.bytes > target {
                        format!(" · above the {target_size} estimate; choose a smaller target to reduce it further")
                    } else { format!(" · target {target_size}") }
                }).unwrap_or_default();
                self.message = Some((format!("Saved {name} · {size}{detail}"), false));
                self.saved = Some(PathBuf::from(clip.path));
            }
            // Cancelling is the person's own choice, not a failure to report.
            Err(text) if text.contains("cancelled") => self.message = None,
            Err(text) => self.message = Some((text, true)),
        }
    }

    /// The trim panel under the controls. Returns a time to jump the player to while a handle is
    /// being dragged, so the picture follows it.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        position: f64,
        duration: f64,
        input: &str,
    ) -> Option<f64> {
        if !self.open {
            return None;
        }
        let bytes = self.source_bytes(input);
        let mut seek = None;
        ui.add_space(12.0);
        ui.painter().hline(
            ui.max_rect().x_range(),
            ui.cursor().top(),
            egui::Stroke::new(1.0_f32, kit::BORDER),
        );
        ui.add_space(12.0);
        let idle = self.job.is_none();
        let (start, end) = self.range(duration);
        let length = end - start;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Trim").color(TEXT).size(15.0).strong());
            ui.add_space(6.0);
            let size = bytes
                .filter(|_| duration > 0.0)
                .map(|bytes| format_bytes((bytes as f64 * length / duration) as u64));
            let summary = match size {
                Some(size) => format!("{length:.1} s  \u{00b7}  \u{2248} {size}"),
                None => format!("{length:.1} s"),
            };
            kit::status_pill(ui, LIME, &summary);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::icon_button(ui, Icon::ChevronDown, 28.0, false, idle, "Close trim (T)")
                    .clicked()
                {
                    self.open = false;
                }
            });
        });
        ui.add_space(8.0);

        let edit = range_bar(
            ui,
            ui.available_width(),
            position,
            duration,
            start,
            end,
            idle,
        );
        if let Some(seconds) = edit.start {
            self.set_start(seconds, duration);
            seek = Some(seconds);
        }
        if let Some(seconds) = edit.end {
            self.set_end(seconds, duration);
            seek = Some(seconds);
        }
        if seek.is_none() {
            seek = edit.seek;
        }
        ui.add_space(10.0);

        ui.add_enabled_ui(idle, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Export").color(MUTED).size(13.0));
                egui::ComboBox::from_id_salt("export-preset")
                    .selected_text(preset_label(self.preset))
                    .show_ui(ui, |ui| {
                        let custom = match self.preset {
                            Some(value @ ExportPreset::Custom { .. }) => value,
                            _ => ExportPreset::Custom { megabytes: 10 },
                        };
                        let options = [
                            (None, "Original quality"),
                            (Some(ExportPreset::Small), "Small file · ~10 MB"),
                            (Some(ExportPreset::Medium), "Medium file · ~25 MB"),
                            (Some(custom), "Custom size"),
                        ];
                        for (value, label) in options {
                            let selected = self.preset == value;
                            let response = ui.selectable_label(selected, label);
                            if response.clicked() {
                                self.preset = value;
                                ui.close();
                            }
                        }
                    });
                if let Some(ExportPreset::Custom { megabytes }) = &mut self.preset {
                    ui.add(egui::DragValue::new(megabytes).range(1..=500).suffix(" MB"));
                }
                let hint = if self.preset.is_some() {
                    "Estimated size. Resolution and frame rate may be reduced."
                } else {
                    "Keeps the original picture and sound quality."
                };
                ui.label(RichText::new(hint).size(12.0).color(MUTED));
            });
        });
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
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
            if idle && (self.start.is_some() || self.end.is_some()) {
                ui.add_space(4.0);
                if ui
                    .add(
                        egui::Label::new(RichText::new("Reset").color(MUTED).size(13.0))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.clear();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(job) = self.job.as_ref() {
                    if kit::labelled_button(ui, Icon::Stop, "Cancel", false, true).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    let fraction = f32::from_bits(job.progress.load(Ordering::Relaxed));
                    ui.add(
                        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                            .desired_width(160.0)
                            .fill(LIME),
                    );
                } else if kit::labelled_button(
                    ui,
                    Icon::Scissors,
                    if self.preset.is_some() { "Export clip" } else { "Save trimmed clip" },
                    true,
                    self.valid(duration),
                )
                .on_hover_text(if self.preset.is_some() {
                    "Saves a compressed MP4 next to the original. The source is preserved. Up to 30 minutes per export."
                } else { "Saves a new MP4 next to the original. Nothing is re-encoded." })
                .clicked()
                {
                    self.save(input, duration, ui.ctx());
                }
            });
        });
        let note = match self.message.as_ref() {
            Some((text, error)) => Some((text.as_str(), if *error { DANGER } else { MUTED })),
            None if self.start.is_none() && self.end.is_none() => Some((
                "Drag the handles, or play to a moment and press I or O.",
                MUTED,
            )),
            None => None,
        };
        if let Some((text, color)) = note {
            ui.add_space(6.0);
            ui.label(RichText::new(text).color(color).size(12.5));
        }
        seek
    }

    /// The size of the clip being trimmed, read once per clip.
    fn source_bytes(&mut self, input: &str) -> Option<u64> {
        if self.size_of.as_ref().is_none_or(|(path, _)| path != input) {
            let bytes = std::fs::metadata(input).ok().map(|meta| meta.len());
            self.size_of = Some((input.to_owned(), bytes));
        }
        self.size_of.as_ref().and_then(|(_, bytes)| *bytes)
    }
}

fn preset_label(preset: Option<ExportPreset>) -> &'static str {
    match preset {
        None => "Original quality",
        Some(ExportPreset::Small) => "Small file · ~10 MB",
        Some(ExportPreset::Medium) => "Medium file · ~25 MB",
        Some(ExportPreset::Custom { .. }) => "Custom size",
    }
}

/// What dragging in the range bar changed.
#[derive(Default)]
struct RangeEdit {
    start: Option<f64>,
    end: Option<f64>,
    seek: Option<f64>,
}

/// The trim bar: the clip as a track, the kept part lit between two handles, the rest dimmed, and
/// the playhead. Dragging a handle moves that end; clicking the track moves the playhead.
fn range_bar(
    ui: &mut egui::Ui,
    width: f32,
    position: f64,
    duration: f64,
    start: f64,
    end: f64,
    enabled: bool,
) -> RangeEdit {
    const HANDLE: f32 = 12.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width.max(0.0), 62.0), egui::Sense::hover());
    let track = egui::Rect::from_min_max(
        egui::pos2(rect.left() + HANDLE, rect.top() + 20.0),
        egui::pos2(rect.right() - HANDLE, rect.bottom() - 2.0),
    );
    let mut edit = RangeEdit::default();
    if duration <= 0.0 || track.width() <= 1.0 {
        ui.painter().rect_filled(track, 8.0, kit::CARD);
        return edit;
    }
    let x_of =
        |seconds: f64| track.left() + track.width() * (seconds / duration).clamp(0.0, 1.0) as f32;
    let time_at = |x: f32| ((x - track.left()) / track.width()).clamp(0.0, 1.0) as f64 * duration;
    let (left, right) = (x_of(start), x_of(end));

    // The track, then a click-to-seek area under the handles.
    let sense = if enabled {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::hover()
    };
    let track_response = ui.interact(track, ui.id().with("trim-track"), sense);
    if let Some(pointer) = track_response
        .interact_pointer_pos()
        .filter(|_| track_response.clicked() || track_response.dragged())
    {
        edit.seek = Some(time_at(pointer.x));
    }
    let painter = ui.painter();
    painter.rect_filled(track, 8.0, kit::CARD);
    painter.rect_stroke(
        track,
        8.0,
        egui::Stroke::new(1.0_f32, kit::BORDER),
        egui::StrokeKind::Inside,
    );
    // Faint ticks every 10 % so the length reads at a glance.
    for tenth in 1..10 {
        let x = track.left() + track.width() * tenth as f32 / 10.0;
        painter.line_segment(
            [
                egui::pos2(x, track.center().y - 4.0),
                egui::pos2(x, track.center().y + 4.0),
            ],
            egui::Stroke::new(1.0_f32, kit::BORDER),
        );
    }
    let kept = egui::Rect::from_min_max(
        egui::pos2(left, track.top()),
        egui::pos2(right.max(left + 1.0), track.bottom()),
    );
    painter.rect_filled(kept, 0.0, LIME.gamma_multiply(0.20));
    painter.line_segment(
        [kept.left_top(), kept.right_top()],
        egui::Stroke::new(2.5_f32, LIME),
    );
    painter.line_segment(
        [kept.left_bottom(), kept.right_bottom()],
        egui::Stroke::new(2.5_f32, LIME),
    );
    let dim = Color32::from_black_alpha(120);
    painter.rect_filled(
        egui::Rect::from_min_max(track.left_top(), egui::pos2(left, track.bottom())),
        8.0,
        dim,
    );
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(right, track.top()), track.right_bottom()),
        8.0,
        dim,
    );

    // The playhead.
    let head = x_of(position);
    painter.line_segment(
        [
            egui::pos2(head, track.top() - 3.0),
            egui::pos2(head, track.bottom() + 1.0),
        ],
        egui::Stroke::new(2.0_f32, Color32::WHITE),
    );

    // The handles, with the time above each.
    let handle =
        |ui: &mut egui::Ui, name: &str, edge: f32, left_side: bool, seconds: f64| -> Option<f64> {
            let body = if left_side {
                egui::Rect::from_min_max(
                    egui::pos2(edge - HANDLE, track.top()),
                    egui::pos2(edge, track.bottom()),
                )
            } else {
                egui::Rect::from_min_max(
                    egui::pos2(edge, track.top()),
                    egui::pos2(edge + HANDLE, track.bottom()),
                )
            };
            let response = ui.interact(
                body.expand2(egui::vec2(6.0, 4.0)),
                ui.id().with(name),
                if enabled {
                    egui::Sense::drag()
                } else {
                    egui::Sense::hover()
                },
            );
            let hot = response.hovered() || response.dragged();
            let rounding = if left_side {
                egui::CornerRadius {
                    nw: 8,
                    sw: 8,
                    ne: 0,
                    se: 0,
                }
            } else {
                egui::CornerRadius {
                    nw: 0,
                    sw: 0,
                    ne: 8,
                    se: 8,
                }
            };
            let painter = ui.painter();
            painter.rect_filled(body, rounding, if hot { Color32::WHITE } else { LIME });
            for dx in [-2.0_f32, 2.0] {
                painter.line_segment(
                    [
                        egui::pos2(body.center().x + dx, body.center().y - 7.0),
                        egui::pos2(body.center().x + dx, body.center().y + 7.0),
                    ],
                    egui::Stroke::new(1.5_f32, Color32::from_rgb(40, 50, 10)),
                );
            }
            painter.text(
                egui::pos2(
                    body.center()
                        .x
                        .clamp(rect.left() + 26.0, rect.right() - 26.0),
                    rect.top() + 4.0,
                ),
                egui::Align2::CENTER_TOP,
                clock(seconds),
                egui::FontId::monospace(11.5),
                if hot { TEXT } else { MUTED },
            );
            if hot && enabled {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            response
                .dragged()
                .then(|| response.interact_pointer_pos())
                .flatten()
                // The pointer holds the middle of the handle; the mark is at its inner edge.
                .map(|pointer| {
                    time_at(
                        pointer.x
                            + if left_side {
                                HANDLE / 2.0
                            } else {
                                -HANDLE / 2.0
                            },
                    )
                })
        };
    edit.start = handle(ui, "trim-start", left, true, start);
    edit.end = handle(ui, "trim-end", right, false, end);
    edit
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
    fn export_controls_draw_at_narrow_and_desktop_sizes_for_each_preset() {
        for width in [300.0, 960.0, 1440.0] {
            for preset in [
                None,
                Some(ExportPreset::Small),
                Some(ExportPreset::Medium),
                Some(ExportPreset::Custom { megabytes: 10 }),
            ] {
                let context = egui::Context::default();
                kit::install_fonts(&context);
                let mut trim = TrimState {
                    open: true,
                    preset,
                    ..Default::default()
                };
                let _ = context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            trim.show(ui, 1.0, 3.0, "missing.mp4");
                        });
                    },
                );
            }
        }
    }

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
