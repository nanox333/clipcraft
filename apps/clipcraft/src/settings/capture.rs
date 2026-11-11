//! The Capture, Video and Audio tabs: quality tiles, encoder choice and the microphone test.

use super::*;

/// What a quality tile says besides its name.
pub(crate) struct TileFacts {
    /// Resolution and frame rate, such as `1080p \u{00b7} 60 FPS`.
    picture: String,
    /// The estimated size of one clip.
    size: String,
    /// The clip length that size is for.
    length: String,
}

/// A tile for picking a quality: its name, the picture, and what a clip weighs. True when
/// clicked.
pub(crate) fn quality_tile(
    ui: &mut egui::Ui,
    width: f32,
    mode: QualityMode,
    selected: bool,
    facts: &TileFacts,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width.max(0.0), 104.0), egui::Sense::click());
    let response = response.on_hover_text(mode.detail());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            true,
            selected,
            format!(
                "{}, about {} per {} clip, {}",
                mode.label(),
                facts.size,
                facts.length,
                facts.picture
            ),
        )
    });
    let fill = if selected {
        Color32::from_rgb(40, 44, 30)
    } else if response.hovered() {
        CARD_HOVER
    } else {
        CARD
    };
    ui.painter().rect_filled(rect, 12.0, fill);
    ui.painter().rect_stroke(
        rect,
        12.0,
        Stroke::new(
            if selected { 1.5_f32 } else { 1.0 },
            if selected { LIME } else { BORDER },
        ),
        egui::StrokeKind::Inside,
    );
    let left = rect.left() + 16.0;
    let write = |y: f32, text: &str, font: egui::FontId, color: Color32| {
        ui.painter().text(
            egui::pos2(left, rect.top() + y),
            egui::Align2::LEFT_TOP,
            text,
            font,
            color,
        );
    };
    write(
        14.0,
        mode.label(),
        egui::FontId::new(15.0, kit::semibold()),
        TEXT,
    );
    write(
        36.0,
        &facts.picture,
        egui::FontId::proportional(12.5),
        MUTED,
    );
    write(
        56.0,
        &format!("\u{2248} {}", facts.size),
        egui::FontId::new(20.0, kit::semibold()),
        if selected { LIME } else { TEXT },
    );
    write(
        82.0,
        &format!("per {} clip", facts.length),
        egui::FontId::proportional(12.0),
        FAINT,
    );
    response.clicked() && !selected
}

pub(crate) fn height_label(height: u32) -> String {
    match height {
        360 => "Low (360p)".to_owned(),
        480 => "SD (480p)".to_owned(),
        720 => "HD (720p)".to_owned(),
        1080 => "Full HD (1080p)".to_owned(),
        1440 => "2K (1440p)".to_owned(),
        2160 => "4K (2160p)".to_owned(),
        other => format!("{other}p"),
    }
}

pub(crate) fn bitrate_label(kbps: u32) -> String {
    if kbps.is_multiple_of(1000) {
        format!("{}M", kbps / 1000)
    } else {
        format!("{:.1}M", kbps as f32 / 1000.0)
    }
}

pub(crate) fn tile_facts(mode: QualityMode, settings: &RecordingSettings) -> TileFacts {
    let (height, fps, kbps) = mode.values().unwrap_or((
        settings.height,
        settings.frames_per_second,
        settings.video_kbps,
    ));
    TileFacts {
        picture: format!("{height}p \u{00b7} {fps} FPS"),
        size: format_bytes(settings.estimated_clip_bytes(kbps)),
        length: clip_length(settings),
    }
}

/// How long a clip is, as shown to people.
pub(crate) fn clip_length(settings: &RecordingSettings) -> String {
    let seconds = settings.clip_seconds();
    if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}

/// One line: clipping keeps no full recording.
pub(crate) fn clip_size_note(ui: &mut egui::Ui, settings: &RecordingSettings) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(format!(
            "Clipping keeps only the last {} in memory. Nothing is saved to disk until you save a clip.",
            clip_length(settings)
        ))
        .color(MUTED)
        .size(12.5),
    )
    .on_hover_text(
        "Sizes are estimated from the bitrate for your clip length (Settings \u{203a} Audio). Variable bitrate can be smaller when the picture is calm.",
    );
}

/// What the chosen video encoder means on this computer.
pub(crate) fn encoder_help(chosen: VideoEncoder, gpu: &GpuProbe) -> String {
    match (chosen, gpu) {
        (VideoEncoder::Cpu, _) => {
            "The processor encodes the video. This works everywhere, but takes processor time from your game."
                .to_owned()
        }
        (VideoEncoder::Gpu, GpuProbe::Ready(name)) => {
            format!("{name} encodes the video, which leaves your processor free for the game.")
        }
        (VideoEncoder::Gpu, GpuProbe::Unavailable(why)) => {
            let _ = why;
            "No graphics-card encoder was found on this computer, so the processor is used."
                .to_owned()
        }
        (VideoEncoder::Gpu, _) => "Checking the graphics card\u{2026}".to_owned(),
    }
}

pub fn capture_tab(
    ui: &mut egui::Ui,
    settings: &mut RecordingSettings,
    targets: &[clipcraft_capture::CaptureTarget],
    refresh: &mut bool,
    capabilities: clipcraft_capture::Capabilities,
) {
    let gpu = GpuProbe::Unknown;
    let mut env = pages::Env {
        gpu: &gpu,
        capabilities,
        targets,
        refresh_targets: refresh,
    };
    let mut rows = crate::rows::Rows::new("");
    pages::capture_rows(&mut rows, ui, settings, &mut env);
}

pub(crate) fn refresh_devices(state: &mut SettingsState) {
    let devices = list_devices();
    state.outputs = devices
        .outputs
        .iter()
        .map(|name| (name.clone(), name.clone()))
        .collect();
    #[cfg(target_os = "linux")]
    if let Ok(found) = clipcraft_platform::list_outputs()
        && !found.outputs.is_empty()
    {
        state.outputs = found
            .outputs
            .into_iter()
            .map(|output| (output.name, output.description))
            .collect();
    }
    state.devices = Some(devices);
}

/// The microphone test: a button and a live level meter.
pub(crate) fn mic_test(ui: &mut egui::Ui, settings: &RecordingSettings, state: &mut SettingsState) {
    state.mic_visible = true;
    ui.horizontal(|ui| {
        let running = state.mic.is_some();
        let label = if running { "Stop test" } else { "Mic Test" };
        if kit::labelled_button(ui, kit::Icon::Speaker, label, !running, true).clicked() {
            if running {
                state.mic = None;
                state.mic_shown = 0.0;
            } else {
                let plan = execute(Command::PrepareMicrophoneTest {
                    settings: Box::new(settings.clone()),
                });
                let started = match plan {
                    Ok(CommandOutput::MicrophoneTest(plan)) => {
                        state.mic_plan = Some(plan.clone());
                        MicTest::start(plan)
                    }
                    _ => Err("The microphone test settings could not be prepared.".into()),
                };
                match started {
                    Ok(test) => {
                        state.mic = Some(test);
                        state.mic_error = None;
                    }
                    Err(error) => state.mic_error = Some(error),
                }
            }
        }
        if state.mic.as_ref().is_some_and(MicTest::finished) {
            state.mic_error = state.mic.as_ref().and_then(MicTest::error);
            state.mic = None;
            state.mic_shown = 0.0;
            ui.ctx().request_repaint();
        }
        let level = state.mic.as_ref().map_or(0.0, MicTest::level);
        state.mic_shown = level.max(state.mic_shown * 0.85);
        let width = (ui.available_width()).max(0.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 28.0), egui::Sense::hover());
        let bars = ((rect.width() / 7.0) as usize).clamp(1, 80);
        let lit = (state.mic_shown.sqrt() * bars as f32).round() as usize;
        for index in 0..bars {
            let x = rect.left() + index as f32 * 7.0;
            let color = if index < lit {
                if index * 10 > bars * 8 { DANGER } else { LIME }
            } else {
                Color32::from_rgb(52, 52, 58)
            };
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, rect.center().y - 9.0),
                    Vec2::new(3.0, 18.0),
                ),
                1.5,
                color,
            );
        }
    });
    if state.mic.is_some() {
        ui.ctx().request_repaint_after(Duration::from_millis(40));
    }
}
