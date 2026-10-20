//! The Settings screens: widgets in the Discord style and one function per tab.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use clipcraft_audio_out::{DeviceNames, MicMeter, list_devices};
use clipcraft_engine::{
    AUDIO_KBPS, CaptureSource, Command, CommandOutput, Container, EncoderPreset, FRAME_RATES,
    Framing, HEIGHTS, HotkeyAction, KEYFRAME_SECONDS, OutputShape, POST_ROLL_SECONDS, QualityMode,
    REPLAY_SECONDS, RateControl, RecordingSettings, SAMPLE_RATES, StorageUsage, VIDEO_KBPS,
    VideoCodec, VideoEncoder, execute, format_bytes, normalize_hotkey,
};
use eframe::egui;
use egui::{Color32, RichText, Stroke, Vec2};

use crate::kit::{self, BORDER, CARD, CARD_HOVER, DANGER, FAINT, LIME, MUTED, TEXT};

/// Whether the graphics card can encode video, as found by trying it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum GpuProbe {
    /// Not tried yet.
    #[default]
    Unknown,
    /// Being tried.
    Checking,
    /// Works; the encoder's name.
    Ready(String),
    /// Does not work; why.
    Unavailable(String),
}

/// What the Settings screens keep between frames.
#[derive(Default)]
pub struct SettingsState {
    devices: Option<DeviceNames>,
    /// Outputs that can be recorded, as (saved name, name shown): PulseAudio's outputs on Linux
    /// (its monitors are what is recorded), the sound system's own list elsewhere.
    outputs: Vec<(String, String)>,
    mic: Option<MicMeter>,
    mic_error: Option<String>,
    mic_shown: f32,
    pub(crate) capturing: Option<HotkeyAction>,
    usage: Option<StorageUsage>,
    usage_for: Option<PathBuf>,
    usage_receiver: Option<mpsc::Receiver<StorageUsage>>,
}

impl SettingsState {
    /// Stops anything running for a screen that is no longer shown.
    pub fn leave(&mut self, keep_mic: bool, keep_capture: bool) {
        if !keep_mic {
            self.mic = None;
            self.mic_shown = 0.0;
        }
        if !keep_capture {
            self.capturing = None;
        }
    }
}

/// One option of a row of choices. True when a different option is clicked. A disabled option
/// explains itself on hover.
pub(crate) fn choice(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    enabled: bool,
    hint: &str,
) -> bool {
    toggle_chip(ui, label, selected, enabled, hint) && !selected
}

/// Like [`choice`], but true on every click, so a selected chip can be switched off.
pub(crate) fn toggle_chip(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    enabled: bool,
    hint: &str,
) -> bool {
    let font = egui::FontId::new(13.5, kit::medium());
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let size = Vec2::new(galley.size().x + 28.0, 34.0);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, selected, label)
    });
    let (fill, stroke, text) = if !enabled {
        (Color32::from_rgb(20, 20, 23), BORDER, FAINT)
    } else if selected {
        (
            Color32::from_rgba_unmultiplied(190, 255, 43, 30),
            LIME,
            LIME,
        )
    } else if response.hovered() {
        (Color32::from_rgb(34, 34, 40), BORDER, TEXT)
    } else {
        (Color32::from_rgb(26, 26, 30), BORDER, TEXT)
    };
    ui.painter().rect_filled(rect, 10.0, fill);
    ui.painter().rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0_f32, stroke),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        text,
    );
    let clicked = response.clone().on_hover_text(hint).clicked();
    clicked && enabled
}

/// A small heading over a group of setting rows.
pub(crate) fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(
        RichText::new(title.to_uppercase())
            .color(FAINT)
            .size(11.5)
            .family(kit::semibold()),
    );
    body(ui);
    ui.add_space(18.0);
}

/// A row with a switch on the right.
pub(crate) fn toggle_row(ui: &mut egui::Ui, title: &str, help: &str, value: &mut bool, last: bool) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let text_width = (ui.available_width() - 56.0).max(0.0);
        ui.vertical(|ui| {
            ui.set_width(text_width);
            ui.label(
                RichText::new(title)
                    .color(TEXT)
                    .size(14.5)
                    .family(kit::semibold()),
            );
            ui.label(RichText::new(help).color(MUTED).size(12.5));
        });
        ui.add_space(12.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, *value, title)
        });
        let fill = if *value {
            LIME
        } else {
            Color32::from_rgb(62, 62, 69)
        };
        ui.painter().rect_filled(rect, 11.0, fill);
        let knob_x = if *value {
            rect.right() - 11.0
        } else {
            rect.left() + 11.0
        };
        ui.painter().circle_filled(
            egui::pos2(knob_x, rect.center().y),
            8.0,
            if *value {
                Color32::BLACK
            } else {
                Color32::WHITE
            },
        );
        if response.on_hover_text(title).clicked() {
            *value = !*value;
        }
    });
    ui.add_space(14.0);
    if !last {
        divider(ui);
    }
}

pub(crate) fn divider(ui: &mut egui::Ui) {
    let (line, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(line, 0.0, BORDER);
}

/// A row with a 0-100 percent slider.
pub(crate) fn percent_row(
    ui: &mut egui::Ui,
    title: &str,
    help: &str,
    value: &mut u32,
    enabled: bool,
) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(title)
            .color(if enabled { TEXT } else { FAINT })
            .size(13.5),
    );
    ui.label(RichText::new(help).color(MUTED).size(12.5));
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 80.0).max(0.0);
        if let Some(next) = kit::slim_slider(ui, width, *value as f32 / 100.0, enabled, title) {
            *value = (next * 100.0).round() as u32;
        }
        ui.add_space(8.0);
        ui.label(RichText::new(format!("{value}%")).color(MUTED));
    });
    ui.add_space(10.0);
}

/// One option of a radio list: a circle, a title, and a description. True when clicked.
pub(crate) fn radio_row(ui: &mut egui::Ui, title: &str, help: &str, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, title)
    });
    let center = egui::pos2(rect.left() + 10.0, rect.top() + 14.0);
    ui.painter().circle_stroke(
        center,
        8.5,
        Stroke::new(1.5_f32, if selected { LIME } else { MUTED }),
    );
    if selected {
        ui.painter().circle_filled(center, 4.5, LIME);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 30.0, rect.top() + 14.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::new(14.0, kit::medium()),
        TEXT,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 30.0, rect.top() + 33.0),
        egui::Align2::LEFT_CENTER,
        help,
        egui::FontId::proportional(12.5),
        MUTED,
    );
    response.clicked() && !selected
}

/// A title and help line, then the row's controls, with a divider after it unless `last`.
pub(crate) fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    help: &str,
    last: bool,
    controls: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(14.0);
    ui.label(
        RichText::new(title)
            .color(TEXT)
            .size(14.5)
            .family(kit::semibold()),
    );
    ui.label(RichText::new(help).color(MUTED).size(12.5));
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        controls(ui);
    });
    ui.add_space(14.0);
    if !last {
        let (line, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().rect_filled(line, 0.0, BORDER);
    }
}

/// A dropdown with its label above it. Returns the index chosen this frame.
fn dropdown(
    ui: &mut egui::Ui,
    label: &str,
    current: &str,
    options: &[(String, bool)],
) -> Option<usize> {
    let mut chosen = None;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.label(RichText::new(label).color(MUTED).size(12.5));
        let shown: String = if current.chars().count() > 30 {
            current
                .chars()
                .take(29)
                .chain(std::iter::once('…'))
                .collect()
        } else {
            current.to_owned()
        };
        kit::pill_menu(ui, &shown, label, |ui| {
            ui.set_min_width(180.0);
            for (index, (text, enabled)) in options.iter().enumerate() {
                let item = ui.add_enabled(*enabled, egui::Button::new(text.as_str()).frame(false));
                if item.clicked() {
                    chosen = Some(index);
                    ui.close();
                }
            }
        });
    });
    chosen
}

/// What a quality tile says besides its name.
struct TileFacts {
    /// Resolution and frame rate, such as `1080p \u{00b7} 60 FPS`.
    picture: String,
    /// The estimated size of one clip.
    size: String,
    /// The clip length that size is for.
    length: String,
}

/// A tile for picking a quality: its name, the picture, and what a clip weighs. True when
/// clicked.
fn quality_tile(
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

fn height_label(height: u32) -> String {
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

fn bitrate_label(kbps: u32) -> String {
    if kbps.is_multiple_of(1000) {
        format!("{}M", kbps / 1000)
    } else {
        format!("{:.1}M", kbps as f32 / 1000.0)
    }
}

fn tile_facts(mode: QualityMode, settings: &RecordingSettings) -> TileFacts {
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
fn clip_length(settings: &RecordingSettings) -> String {
    let seconds = settings.clip_seconds();
    if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}

/// One line: clipping keeps no full recording.
fn clip_size_note(ui: &mut egui::Ui, settings: &RecordingSettings) {
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

pub fn quality_tab(ui: &mut egui::Ui, settings: &mut RecordingSettings, gpu: &GpuProbe) {
    card(ui, "Recording quality", |ui| {
        ui.add_space(10.0);
        let gap = 10.0;
        let width = (ui.available_width() - gap * 3.0) / 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for mode in QualityMode::ALL {
                let selected = settings.quality == mode;
                if quality_tile(ui, width, mode, selected, &tile_facts(mode, settings)) {
                    settings.quality = mode;
                }
            }
        });
        clip_size_note(ui, settings);
        if settings.quality == QualityMode::Custom {
            ui.add_space(14.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(14.0, 12.0);
                let heights: Vec<(String, bool)> =
                    HEIGHTS.iter().map(|h| (height_label(*h), true)).collect();
                if let Some(index) =
                    dropdown(ui, "Resolution", &height_label(settings.height), &heights)
                {
                    settings.height = HEIGHTS[index];
                }
                let rates: Vec<(String, bool)> =
                    FRAME_RATES.iter().map(|r| (r.to_string(), true)).collect();
                if let Some(index) =
                    dropdown(ui, "FPS", &settings.frames_per_second.to_string(), &rates)
                {
                    settings.frames_per_second = FRAME_RATES[index];
                }
                let bitrates: Vec<(String, bool)> = VIDEO_KBPS
                    .iter()
                    .map(|k| (bitrate_label(*k), true))
                    .collect();
                if let Some(index) = dropdown(
                    ui,
                    "Bitrate",
                    &bitrate_label(settings.video_kbps),
                    &bitrates,
                ) {
                    settings.video_kbps = VIDEO_KBPS[index];
                }
            });
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(14.0, 12.0);
                let gpu_label = match gpu {
                    GpuProbe::Ready(name) => format!("GPU ({name})"),
                    GpuProbe::Checking | GpuProbe::Unknown => "GPU (checking\u{2026})".to_owned(),
                    GpuProbe::Unavailable(_) => "GPU (unavailable)".to_owned(),
                };
                let encoders = vec![
                    ("CPU".to_owned(), true),
                    (gpu_label.clone(), matches!(gpu, GpuProbe::Ready(_))),
                ];
                let shown = match settings.video_encoder {
                    VideoEncoder::Cpu => "CPU".to_owned(),
                    VideoEncoder::Gpu => gpu_label,
                };
                if let Some(index) = dropdown(ui, "Video Encoder", &shown, &encoders) {
                    settings.video_encoder = VideoEncoder::ALL[index];
                }
                let codecs: Vec<(String, bool)> = VideoCodec::ALL
                    .iter()
                    .map(|c| (c.label().to_owned(), c.unavailable_reason().is_none()))
                    .collect();
                if let Some(index) = dropdown(ui, "Codec", settings.codec.label(), &codecs) {
                    settings.codec = VideoCodec::ALL[index];
                }
            });
            ui.add_space(8.0);
            let (note, color) = match gpu {
                GpuProbe::Ready(name) => (
                    format!(
                        "{name} encodes the video, which leaves your processor free for the game. H.265 is not available yet."
                    ),
                    FAINT,
                ),
                GpuProbe::Unavailable(why) => (
                    format!(
                        "GPU encoding is not available on this computer: {why}. Recording uses the CPU. H.265 is not available yet."
                    ),
                    FAINT,
                ),
                _ => ("Checking the graphics card\u{2026}".to_owned(), FAINT),
            };
            ui.label(RichText::new(note).color(color).size(12.0));
        }
        ui.add_space(14.0);
    });
    card(ui, "File", |ui| {
        setting_row(ui, "Format", settings.container.detail(), false, |ui| {
            for container in Container::ALL {
                let reason = container.unavailable_reason();
                let hint = reason.map_or_else(|| container.detail().to_owned(), str::to_owned);
                if choice(
                    ui,
                    container.label(),
                    settings.container == container,
                    reason.is_none(),
                    &hint,
                ) {
                    settings.container = container;
                }
            }
        });
        setting_row(
            ui,
            "Bitrate mode",
            settings.rate_control.detail(),
            true,
            |ui| {
                for mode in RateControl::ALL {
                    if choice(
                        ui,
                        mode.label(),
                        settings.rate_control == mode,
                        true,
                        mode.detail(),
                    ) {
                        settings.rate_control = mode;
                    }
                }
            },
        );
    });
}

pub fn capture_tab(
    ui: &mut egui::Ui,
    settings: &mut RecordingSettings,
    targets: &[clipcraft_capture::CaptureTarget],
    refresh: &mut bool,
    capabilities: clipcraft_capture::Capabilities,
) {
    card(ui, "What to record", |ui| {
        setting_row(ui, "Capture", settings.source.detail(), false, |ui| {
            for source in CaptureSource::ALL {
                if choice(
                    ui,
                    source.label(),
                    settings.source == source,
                    true,
                    source.detail(),
                ) {
                    settings.source = source;
                }
            }
        });
        let want_window = settings.source == CaptureSource::Game;
        let kind = if want_window {
            clipcraft_capture::TargetKind::Window
        } else {
            clipcraft_capture::TargetKind::Display
        };
        let choices: Vec<&clipcraft_capture::CaptureTarget> = targets
            .iter()
            .filter(|target| target.kind == kind)
            .collect();
        let current = match choices
            .iter()
            .find(|target| target.id == settings.capture_target)
        {
            Some(target) => target.name.clone(),
            None if !settings.capture_target.is_empty() => {
                format!("{} (not open)", settings.capture_target_name)
            }
            None if want_window => "Choose a window".to_owned(),
            None => "Main monitor".to_owned(),
        };
        let mut options = vec![(
            if want_window {
                "None chosen".to_owned()
            } else {
                "Main monitor".to_owned()
            },
            true,
        )];
        options.extend(choices.iter().map(|target| {
            (
                format!("{}  ({}\u{d7}{})", target.name, target.width, target.height),
                true,
            )
        }));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if let Some(index) = dropdown(
                ui,
                if want_window {
                    "Game window"
                } else {
                    "Monitor"
                },
                &current,
                &options,
            ) {
                match index.checked_sub(1).and_then(|at| choices.get(at)) {
                    Some(target) => {
                        settings.capture_target = target.id.clone();
                        settings.capture_target_name = target.name.clone();
                    }
                    None => {
                        settings.capture_target.clear();
                        settings.capture_target_name.clear();
                    }
                }
            }
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.add_space(21.0);
                if kit::labelled_button(ui, kit::Icon::Loop, "Refresh", false, true).clicked() {
                    *refresh = true;
                }
            });
        });
        if want_window && choices.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new("No windows were found. Open the game, then press Refresh.")
                    .color(FAINT)
                    .size(12.5),
            );
        }
        ui.add_space(10.0);
        divider(ui);
        toggle_row(
            ui,
            "Start clipping when ClipCraft opens",
            "Starts filling the replay buffer as soon as the app starts.",
            &mut settings.record_on_launch,
            false,
        );
        toggle_row(
            ui,
            "Show the mouse cursor",
            "Draw the pointer in your clips.",
            &mut settings.capture_cursor,
            false,
        );
        toggle_row(
            ui,
            "Force window capture",
            if capabilities.windows {
                "Record the window that has the focus (your game) even when you picked a whole monitor, and never the monitor around it. Start clipping with its hotkey while the game is in front. Try this if a game records black."
            } else {
                "Window capture is not available on this computer."
            },
            &mut settings.force_window_capture,
            false,
        );
        toggle_row(
            ui,
            "HDR compatibility",
            if capabilities.hdr {
                "Convert an HDR monitor to standard colours, with highlights rolled off instead of clipped, so clips are not washed out when shared. Does nothing on a standard monitor."
            } else {
                "This computer's capture does not report HDR, so this has no effect here."
            },
            &mut settings.hdr_compatibility,
            true,
        );
    });
}

fn refresh_devices(state: &mut SettingsState) {
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

fn device_dropdown(ui: &mut egui::Ui, label: &str, current: &mut String, names: &[String]) {
    let mut options = vec![("System default".to_owned(), true)];
    options.extend(names.iter().map(|name| (name.clone(), true)));
    let shown = if current.is_empty() {
        "System default"
    } else {
        current.as_str()
    };
    let shown = shown.to_owned();
    if let Some(index) = dropdown(ui, label, &shown, &options) {
        *current = if index == 0 {
            String::new()
        } else {
            names[index - 1].clone()
        };
    }
}

/// Like [`device_dropdown`], for devices whose saved name differs from the name shown.
fn output_dropdown(
    ui: &mut egui::Ui,
    label: &str,
    current: &mut String,
    outputs: &[(String, String)],
) {
    let mut options = vec![("System default".to_owned(), true)];
    options.extend(outputs.iter().map(|(_, shown)| (shown.clone(), true)));
    let shown = outputs
        .iter()
        .find(|(name, _)| name == current)
        .map_or_else(
            || {
                if current.is_empty() {
                    "System default".to_owned()
                } else {
                    current.clone()
                }
            },
            |(_, shown)| shown.clone(),
        );
    if let Some(index) = dropdown(ui, label, &shown, &options) {
        *current = if index == 0 {
            String::new()
        } else {
            outputs[index - 1].0.clone()
        };
    }
}

pub fn audio_tab(ui: &mut egui::Ui, settings: &mut RecordingSettings, state: &mut SettingsState) {
    if state.devices.is_none() {
        refresh_devices(state);
    }
    let devices = state.devices.clone().unwrap_or_default();
    card(ui, "PC audio", |ui| {
        toggle_row(
            ui,
            "Record game and system sound",
            "Everything you hear from the output device below.",
            &mut settings.capture_game_audio,
            false,
        );
        ui.add_space(4.0);
        output_dropdown(
            ui,
            "Output device to record",
            &mut settings.output_device,
            &state.outputs,
        );
        ui.add_space(4.0);
        percent_row(
            ui,
            "Game volume",
            "How loud the game is in the clip.",
            &mut settings.game_audio_percent,
            settings.capture_game_audio,
        );
    });
    card(ui, "Microphone", |ui| {
        toggle_row(
            ui,
            "Record microphone",
            "Your voice goes into the clip.",
            &mut settings.capture_microphone,
            false,
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            device_dropdown(
                ui,
                "Microphone",
                &mut settings.microphone_device,
                &devices.inputs,
            );
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.add_space(21.0);
                if kit::labelled_button(ui, kit::Icon::Loop, "Refresh", false, true).clicked() {
                    refresh_devices(state);
                }
            });
        });
        ui.add_space(8.0);
        mic_test(ui, settings, state);
        ui.add_space(8.0);
        divider(ui);
        percent_row(
            ui,
            "Microphone volume",
            "How loud your voice is in the clip.",
            &mut settings.microphone_percent,
            settings.capture_microphone,
        );
        divider(ui);
        toggle_row(
            ui,
            "Microphone is mono",
            "Copy a one-channel microphone to both sides so you are not heard from one ear.",
            &mut settings.microphone_mono,
            false,
        );
        toggle_row(
            ui,
            "Push to talk",
            "Your microphone is only recorded while you hold the push-to-talk key.",
            &mut settings.push_to_talk,
            true,
        );
        if settings.push_to_talk && settings.hotkeys.get(HotkeyAction::PushToTalk).is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new("No push-to-talk key is set yet. Choose one in Hotkeys.")
                    .color(DANGER)
                    .size(12.5),
            );
        }
        ui.add_space(8.0);
    });
    card(ui, "Tracks", |ui| {
        toggle_row(
            ui,
            "Separate audio tracks",
            "Save game sound and microphone as their own tracks so you can edit them apart. Without this they are mixed into one.",
            &mut settings.separate_audio_tracks,
            true,
        );
    });
    card(ui, "Quality", |ui| {
        setting_row(
            ui,
            "Audio bitrate",
            "AAC audio, kilobits per second. 256 and above is transparent for most listeners.",
            false,
            |ui| {
                for kbps in AUDIO_KBPS {
                    if choice(
                        ui,
                        &format!("{kbps} kbps"),
                        settings.audio_kbps == kbps,
                        true,
                        "Audio bitrate",
                    ) {
                        settings.audio_kbps = kbps;
                    }
                }
            },
        );
        setting_row(
            ui,
            "Instant replay length",
            "How much play before the hotkey is kept.",
            true,
            |ui| {
                for seconds in REPLAY_SECONDS {
                    let label = if seconds >= 60 {
                        format!("{} min", seconds / 60)
                    } else {
                        format!("{seconds} sec")
                    };
                    if choice(
                        ui,
                        &label,
                        settings.replay_seconds == seconds,
                        true,
                        "Replay length",
                    ) {
                        settings.replay_seconds = seconds;
                    }
                }
            },
        );
        setting_row(
            ui,
            "Keep recording after the hotkey",
            "Adds the moments after you press the key, such as the reaction. The clip is saved once they have been recorded.",
            true,
            |ui| {
                for seconds in POST_ROLL_SECONDS {
                    let label = if seconds == 0 {
                        "Off".to_owned()
                    } else {
                        format!("{seconds} sec")
                    };
                    if choice(
                        ui,
                        &label,
                        settings.post_roll_seconds == seconds,
                        true,
                        "Post-roll length",
                    ) {
                        settings.post_roll_seconds = seconds;
                    }
                }
            },
        );
    });
}

/// The microphone test: a button and a live level meter.
fn mic_test(ui: &mut egui::Ui, settings: &RecordingSettings, state: &mut SettingsState) {
    ui.horizontal(|ui| {
        let running = state.mic.is_some();
        let label = if running { "Stop test" } else { "Mic Test" };
        if kit::labelled_button(ui, kit::Icon::Speaker, label, !running, true).clicked() {
            if running {
                state.mic = None;
                state.mic_shown = 0.0;
            } else {
                match MicMeter::start(&settings.microphone_device) {
                    Ok(meter) => {
                        state.mic = Some(meter);
                        state.mic_error = None;
                    }
                    Err(error) => state.mic_error = Some(error.to_string()),
                }
            }
        }
        if state.mic.as_ref().is_some_and(MicMeter::failed) {
            state.mic = None;
            state.mic_error = Some("The microphone stopped.".to_owned());
        }
        let level = state.mic.as_ref().map_or(0.0, MicMeter::level);
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
    if let Some(error) = &state.mic_error {
        ui.add_space(4.0);
        ui.label(RichText::new(error).color(DANGER).size(12.5));
    } else if state.mic.is_some() {
        ui.add_space(4.0);
        ui.label(
            RichText::new("Speak: the bars should move.")
                .color(MUTED)
                .size(12.5),
        );
    }
}

fn key_for_egui(key: egui::Key) -> Option<&'static str> {
    use egui::Key as K;
    Some(match key {
        K::ArrowDown => "ArrowDown",
        K::ArrowLeft => "ArrowLeft",
        K::ArrowRight => "ArrowRight",
        K::ArrowUp => "ArrowUp",
        K::Backspace => "Backspace",
        K::Delete => "Delete",
        K::End => "End",
        K::Enter => "Enter",
        K::Home => "Home",
        K::Insert => "Insert",
        K::PageDown => "PageDown",
        K::PageUp => "PageUp",
        K::Space => "Space",
        K::Tab => "Tab",
        K::Minus => "Minus",
        K::Equals => "Equals",
        K::Comma => "Comma",
        K::Period => "Period",
        K::Slash => "Slash",
        K::Backtick => "Backtick",
        _ => return None,
    })
}

/// The shortcut being pressed this frame, in canonical text, if any.
pub(crate) fn pressed_shortcut_in(input: &egui::InputState) -> Option<String> {
    input.events.iter().find_map(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = event
        else {
            return None;
        };
        let name = key_for_egui(*key).map_or_else(|| key.name().to_owned(), str::to_owned);
        let mut text = String::new();
        for (on, label) in [
            (modifiers.ctrl || modifiers.command, "Ctrl"),
            (modifiers.alt, "Alt"),
            (modifiers.shift, "Shift"),
        ] {
            if on {
                text.push_str(label);
                text.push('+');
            }
        }
        text.push_str(&name);
        normalize_hotkey(&text)
    })
}

fn pressed_shortcut(ui: &egui::Ui) -> Option<String> {
    ui.input(pressed_shortcut_in)
}

pub fn hotkeys_tab(ui: &mut egui::Ui, settings: &mut RecordingSettings, state: &mut SettingsState) {
    if state.capturing.is_some() {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            state.capturing = None;
        } else if let (Some(action), Some(shortcut)) = (state.capturing, pressed_shortcut(ui)) {
            settings.hotkeys.set(action, &shortcut);
            state.capturing = None;
        }
    }
    card(ui, "Shortcuts", |ui| {
        let count = HotkeyAction::ALL.len();
        for (index, action) in HotkeyAction::ALL.into_iter().enumerate() {
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let text_width = (ui.available_width() - 200.0).max(0.0);
                ui.vertical(|ui| {
                    ui.set_width(text_width);
                    ui.label(
                        RichText::new(action.label())
                            .color(TEXT)
                            .size(14.5)
                            .family(kit::semibold()),
                    );
                    ui.label(RichText::new(action.detail()).color(MUTED).size(12.5));
                    if let Some(other) = settings.hotkeys.conflict(action) {
                        ui.label(
                            RichText::new(format!("Also used by “{}”.", other.label()))
                                .color(DANGER)
                                .size(12.5),
                        );
                    }
                });
                ui.add_space(8.0);
                let waiting = state.capturing == Some(action);
                let current = settings.hotkeys.get(action).to_owned();
                let text = if waiting {
                    "Press a key…".to_owned()
                } else if current.is_empty() {
                    "Not set".to_owned()
                } else {
                    current.replace('+', " + ")
                };
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(130.0, 34.0), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 8.0, Color32::from_rgb(13, 13, 15));
                ui.painter().rect_stroke(
                    rect,
                    8.0,
                    Stroke::new(1.0_f32, if waiting { LIME } else { BORDER }),
                    egui::StrokeKind::Inside,
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::new(13.5, kit::medium()),
                    if waiting {
                        LIME
                    } else if current.is_empty() {
                        FAINT
                    } else {
                        TEXT
                    },
                );
                if response
                    .on_hover_text("Click, then press the keys")
                    .clicked()
                {
                    state.capturing = if waiting { None } else { Some(action) };
                }
                let clear = ui.add_enabled(
                    !current.is_empty(),
                    egui::Button::new(RichText::new("Clear").color(MUTED)).frame(false),
                );
                if clear.clicked() {
                    settings.hotkeys.set(action, "");
                }
            });
            ui.add_space(14.0);
            if index + 1 < count {
                divider(ui);
            }
        }
    });
    ui.label(
        RichText::new("Click a shortcut, then press the keys you want. Press Esc to cancel.")
            .color(FAINT)
            .size(12.5),
    );
    ui.add_space(12.0);
    if kit::labelled_button(ui, kit::Icon::Loop, "Reset to defaults", false, true).clicked() {
        settings.hotkeys = clipcraft_engine::Hotkeys::default();
        state.capturing = None;
    }
    ui.add_space(8.0);
    if state.capturing.is_some() {
        ui.ctx().request_repaint();
    }
}

fn request_usage(state: &mut SettingsState, context: &egui::Context, folder: PathBuf, force: bool) {
    if !force && state.usage_for.as_ref() == Some(&folder) {
        return;
    }
    state.usage_for = Some(folder.clone());
    let (sender, receiver) = mpsc::channel();
    state.usage_receiver = Some(receiver);
    let context = context.clone();
    std::thread::spawn(move || {
        if let Ok(CommandOutput::StorageUsage(usage)) = execute(Command::StorageUsage { folder }) {
            let _ = sender.send(usage);
            context.request_repaint();
        }
    });
}

/// Paints a horizontal bar filled to `fraction`.
fn usage_bar(ui: &mut egui::Ui, fraction: f32, warn: bool) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 5.0, Color32::from_rgb(52, 52, 58));
    let filled = egui::Rect::from_min_size(
        rect.min,
        Vec2::new(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
    );
    ui.painter()
        .rect_filled(filled, 5.0, if warn { DANGER } else { LIME });
}

pub fn storage_tab(
    ui: &mut egui::Ui,
    settings: &mut RecordingSettings,
    state: &mut SettingsState,
    folder: Option<PathBuf>,
    browse: &mut bool,
) {
    if let Some(folder) = folder.clone() {
        request_usage(state, ui.ctx(), folder, false);
    }
    if let Some(receiver) = &state.usage_receiver
        && let Ok(usage) = receiver.try_recv()
    {
        state.usage = Some(usage);
    }
    card(ui, "Clips folder", |ui| {
        ui.add_space(14.0);
        ui.label(
            RichText::new("Where clips are saved")
                .color(TEXT)
                .size(14.5)
                .family(kit::semibold()),
        );
        ui.label(
            RichText::new(
                "New clips go here, and this is the folder your library shows. Existing clips are not moved.",
            )
            .color(MUTED)
            .size(12.5),
        );
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(13, 13, 15))
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 34.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        let (icon, _) =
                            ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
                        kit::draw_icon(ui.painter(), icon.center(), kit::Icon::Folder, MUTED, 0.8);
                        let shown = folder.as_ref().map_or_else(
                            || "No folder available".to_owned(),
                            |f| f.to_string_lossy().into_owned(),
                        );
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(RichText::new(shown).color(TEXT)).truncate());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            *browse =
                                kit::labelled_button(ui, kit::Icon::Folder, "Browse", false, true)
                                    .clicked();
                        });
                    },
                );
            });
        ui.add_space(14.0);
    });
    card(ui, "Storage used", |ui| {
        ui.add_space(14.0);
        let limit_bytes = u64::from(settings.storage_limit_gb) * 1_000_000_000;
        match &state.usage {
            Some(usage) => {
                let used = crate::format_bytes(usage.used_bytes);
                let plus = if usage.truncated { "+" } else { "" };
                let clips = format!(
                    "{used}{plus} used by {} clip{}",
                    usage.clip_count,
                    if usage.clip_count == 1 { "" } else { "s" }
                );
                ui.label(
                    RichText::new(clips)
                        .color(TEXT)
                        .size(14.5)
                        .family(kit::semibold()),
                );
                ui.add_space(8.0);
                if limit_bytes > 0 {
                    usage_bar(
                        ui,
                        usage.used_bytes as f32 / limit_bytes as f32,
                        usage.used_bytes > limit_bytes,
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "{} of your {} GB limit",
                            crate::format_bytes(usage.used_bytes),
                            settings.storage_limit_gb
                        ))
                        .color(MUTED)
                        .size(12.5),
                    );
                } else if let Some(total) = usage.total_bytes.filter(|total| *total > 0) {
                    usage_bar(ui, usage.used_bytes as f32 / total as f32, false);
                }
                ui.add_space(10.0);
                if let (Some(free), Some(total)) = (usage.free_bytes, usage.total_bytes) {
                    ui.label(
                        RichText::new(format!(
                            "{} available of {} on this disk",
                            crate::format_bytes(free),
                            crate::format_bytes(total)
                        ))
                        .color(MUTED)
                        .size(12.5),
                    );
                }
                if !usage.exists {
                    ui.label(
                        RichText::new("The folder does not exist yet. It is created when the first clip is saved.")
                            .color(FAINT)
                            .size(12.5),
                    );
                }
            }
            None => {
                ui.label(RichText::new("Measuring…").color(MUTED));
            }
        }
        ui.add_space(10.0);
        if kit::labelled_button(ui, kit::Icon::Loop, "Refresh", false, true).clicked()
            && let Some(folder) = folder.clone()
        {
            request_usage(state, ui.ctx(), folder, true);
        }
        ui.add_space(14.0);
    });
    card(ui, "Storage limit", |ui| {
        setting_row(
            ui,
            "Most space clips may use",
            "ClipCraft stops saving new clips, or deletes the oldest, when this is reached.",
            false,
            |ui| {
                for gigabytes in [0_u32, 10, 25, 50, 100, 250, 500, 1000] {
                    let label = match gigabytes {
                        0 => "Unlimited".to_owned(),
                        1000 => "1 TB".to_owned(),
                        other => format!("{other} GB"),
                    };
                    if choice(
                        ui,
                        &label,
                        settings.storage_limit_gb == gigabytes,
                        true,
                        "Storage limit",
                    ) {
                        settings.storage_limit_gb = gigabytes;
                    }
                }
            },
        );
        ui.add_space(8.0);
        let enabled = settings.storage_limit_gb > 0;
        ui.add_enabled_ui(enabled, |ui| {
            if radio_row(
                ui,
                "Delete the oldest clips",
                "Keeps recording by making room.",
                settings.delete_oldest_when_full,
            ) {
                settings.delete_oldest_when_full = true;
            }
            if radio_row(
                ui,
                "Stop saving new clips",
                "Nothing is ever deleted for you.",
                !settings.delete_oldest_when_full,
            ) {
                settings.delete_oldest_when_full = false;
            }
        });
        ui.add_space(8.0);
    });
}

/// The About tab: what ClipCraft is, which version this is, and where to find the project.
pub fn about_tab(ui: &mut egui::Ui) {
    card(ui, "ClipCraft", |ui| {
        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "ClipCraft records your monitor or game window and keeps the last moments ready to save. Press \
                 a hotkey and the last seconds become a clip. Then find, play, trim and keep your \
                 clips from one library.",
            )
            .color(TEXT)
            .size(14.5),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Everything happens on this computer. ClipCraft has no account, no upload, no \
                 sharing service and no tracking, and it works without a network connection.",
            )
            .color(MUTED)
            .size(13.5),
        );
        ui.add_space(14.0);
    });
    card(ui, "Details", |ui| {
        let version = format!(
            "{} ({} {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        for (title, value) in [
            ("Version", version.as_str()),
            ("License", "MIT or Apache-2.0, open source"),
        ] {
            ui.add_space(10.0);
            ui.label(
                RichText::new(title)
                    .color(TEXT)
                    .size(14.5)
                    .family(kit::semibold()),
            );
            ui.label(RichText::new(value).color(MUTED).size(12.5));
        }
        ui.add_space(6.0);
    });
    card(ui, "Project", |ui| {
        let url = "https://github.com/nanox333/clipcraft";
        ui.add_space(8.0);
        ui.add(egui::Hyperlink::from_label_and_url(
            RichText::new("Source code and issues")
                .color(kit::LIME)
                .size(14.0),
            url,
        ));
        ui.label(RichText::new(url).color(FAINT).size(12.0));
        ui.add_space(8.0);
    });
}

pub fn advanced_tab(ui: &mut egui::Ui, settings: &mut RecordingSettings) {
    card(ui, "Video aspect ratio", |ui| {
        ui.add_space(8.0);
        let current = match (settings.shape, settings.framing) {
            (OutputShape::Source, _) => 0,
            (_, Framing::Letterbox) => 2,
            (_, Framing::Crop) => 3,
            _ => 1,
        };
        let options = [
            (
                "Game aspect ratio",
                "Clips keep the game's own shape, 4:3 stays 4:3.",
            ),
            (
                "Stretch to fit 16:9",
                "Fills a widescreen frame. A 4:3 game looks wider.",
            ),
            (
                "Fit 16:9 with black bars",
                "Keeps the shape and fills the sides with black.",
            ),
            (
                "Crop to 16:9",
                "Keeps the shape and trims the top and bottom.",
            ),
        ];
        for (index, (title, help)) in options.into_iter().enumerate() {
            if radio_row(ui, title, help, current == index) {
                match index {
                    0 => {
                        settings.shape = OutputShape::Source;
                        settings.framing = Framing::Original;
                    }
                    1 => {
                        settings.shape = OutputShape::Wide16x9;
                        settings.framing = Framing::Stretch;
                    }
                    2 => {
                        settings.shape = OutputShape::Wide16x9;
                        settings.framing = Framing::Letterbox;
                    }
                    _ => {
                        settings.shape = OutputShape::Wide16x9;
                        settings.framing = Framing::Crop;
                    }
                }
            }
        }
        ui.add_space(8.0);
    });
    card(ui, "Encoder", |ui| {
        ui.add_space(8.0);
        for preset in EncoderPreset::ALL {
            if radio_row(
                ui,
                preset.label(),
                preset.detail(),
                settings.encoder_preset == preset,
            ) {
                settings.encoder_preset = preset;
            }
        }
        divider(ui);
        setting_row(
            ui,
            "Keyframe interval",
            "Seconds between keyframes. Shorter clips scrub faster; longer ones are smaller.",
            true,
            |ui| {
                for seconds in KEYFRAME_SECONDS {
                    if choice(
                        ui,
                        &format!("{seconds} s"),
                        settings.keyframe_seconds == seconds,
                        true,
                        "Keyframe interval",
                    ) {
                        settings.keyframe_seconds = seconds;
                    }
                }
            },
        );
    });
    card(ui, "Audio", |ui| {
        setting_row(
            ui,
            "Sample rate",
            "48 kHz is the video standard.",
            false,
            |ui| {
                for rate in SAMPLE_RATES {
                    if choice(
                        ui,
                        &format!("{} kHz", rate as f32 / 1000.0),
                        settings.audio_sample_rate == rate,
                        true,
                        "Sample rate",
                    ) {
                        settings.audio_sample_rate = rate;
                    }
                }
            },
        );
        setting_row(
            ui,
            "Channels",
            "Stereo keeps left and right apart.",
            true,
            |ui| {
                for (channels, label) in [(2, "Stereo"), (1, "Mono")] {
                    if choice(ui, label, settings.audio_channels == channels, true, label) {
                        settings.audio_channels = channels;
                    }
                }
            },
        );
    });
    card(ui, "Advanced settings", |ui| {
        toggle_row(
            ui,
            "Show advanced settings",
            "Turn off to hide this section again.",
            &mut settings.show_advanced,
            true,
        );
    });
}

/// The start-up switches shown on the General tab.
pub fn startup_card(ui: &mut egui::Ui, settings: &mut RecordingSettings) {
    card(ui, "Startup", |ui| {
        toggle_row(
            ui,
            "Launch ClipCraft when I sign in",
            "Starts quietly in the background so clips are ready when you play.",
            &mut settings.launch_at_startup,
            false,
        );
        toggle_row(
            ui,
            "Start minimized",
            "Open without showing the window.",
            &mut settings.start_minimized,
            true,
        );
    });
}
