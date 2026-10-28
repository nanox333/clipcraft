//! The Settings pages built from rows (see `rows`): Overview, Capture, Video, Audio, Replay,
//! Storage and General. Each page is a list of rows, so the search box can draw any of them in
//! place and every row can show that it was changed and be put back.

use super::*;
use crate::rows::{Meta, Rows};

/// Everything a page needs to know about the computer, besides the settings themselves.
pub(crate) struct Env<'a> {
    pub gpu: &'a GpuProbe,
    pub capabilities: clipcraft_capture::Capabilities,
    pub targets: &'a [clipcraft_capture::CaptureTarget],
    pub refresh_targets: &'a mut bool,
}

// ---------------------------------------------------------------------------------------------
// Overview
// ---------------------------------------------------------------------------------------------

/// How recording is going right now, for the Overview.
pub(crate) struct Live {
    pub recording: bool,
    pub captured: u64,
    pub dropped: u64,
    pub encode_ms: f32,
    pub target_fps: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Good,
    Note,
    Warn,
}

impl Tone {
    fn color(self) -> Color32 {
        match self {
            Self::Good => LIME,
            Self::Note => Color32::from_rgb(120, 170, 255),
            Self::Warn => Color32::from_rgb(255, 196, 77),
        }
    }
}

/// A button on a suggestion: its label and what it changes.
type Action = (&'static str, fn(&mut RecordingSettings));

struct Insight {
    tone: Tone,
    title: String,
    body: String,
    action: Option<Action>,
}

fn use_gpu(settings: &mut RecordingSettings) {
    settings.video_encoder = VideoEncoder::Gpu;
}

fn lighter_picture(settings: &mut RecordingSettings) {
    settings.quality = QualityMode::Standard;
    settings.encoder_preset = EncoderPreset::Fast;
}

fn lower_effort(settings: &mut RecordingSettings) {
    settings.encoder_preset = EncoderPreset::Fast;
}

fn insights(settings: &RecordingSettings, gpu: &GpuProbe, live: &Live) -> Vec<Insight> {
    let mut found = Vec::new();
    let heavy = settings.height >= 1080 && settings.frames_per_second >= 60;
    if settings.video_encoder == VideoEncoder::Cpu {
        if let GpuProbe::Ready(name) = gpu {
            found.push(Insight {
                tone: Tone::Warn,
                title: "Your graphics card can do the encoding".to_owned(),
                body: format!(
                    "The processor is encoding your video. {name} is available and leaves the processor free for your game."
                ),
                action: Some(("Use the graphics card", use_gpu)),
            });
        } else if heavy {
            found.push(Insight {
                tone: Tone::Warn,
                title: "The processor may not keep up with 1080p at 60 FPS".to_owned(),
                body: "Frames get dropped when encoding takes longer than one frame. A lighter picture and the Fast effort setting help.".to_owned(),
                action: Some(("Use a lighter picture", lighter_picture)),
            });
        }
    }
    if live.recording && live.captured > 120 {
        let percent = live.dropped as f32 * 100.0 / live.captured.max(1) as f32;
        let budget = 1000.0 / live.target_fps.max(1) as f32;
        if percent > 2.0 {
            found.push(Insight {
                tone: Tone::Warn,
                title: format!("{percent:.1}% of frames are being dropped"),
                body: format!(
                    "Encoding takes {:.1} ms a frame and one frame lasts {budget:.1} ms. Lower the frame rate or resolution, or ease the encoder effort.",
                    live.encode_ms
                ),
                action: (settings.encoder_preset != EncoderPreset::Fast)
                    .then_some(("Use Fast effort", lower_effort as fn(&mut RecordingSettings))),
            });
        }
    }
    if settings.encoder_preset == EncoderPreset::Quality
        && settings.video_encoder == VideoEncoder::Cpu
    {
        found.push(Insight {
            tone: Tone::Note,
            title: "Quality effort is the slowest on the processor".to_owned(),
            body: "It makes the smallest files but is the most likely to drop frames in a game."
                .to_owned(),
            action: Some(("Use Fast effort", lower_effort)),
        });
    }
    if settings.codec == VideoCodec::H265 && settings.video_encoder == VideoEncoder::Cpu {
        found.push(Insight {
            tone: Tone::Warn,
            title: "H.265 needs the graphics card".to_owned(),
            body: "The processor encoder only makes H.264, so your clips are H.264 until you switch the encoder."
                .to_owned(),
            action: Some(("Use the graphics card", use_gpu)),
        });
    }
    if found.is_empty() {
        found.push(Insight {
            tone: Tone::Good,
            title: "Everything looks good".to_owned(),
            body: if live.recording {
                format!(
                    "Recording at {} FPS, encoding in {:.1} ms a frame, {} frames dropped.",
                    live.target_fps, live.encode_ms, live.dropped
                )
            } else {
                "Your video settings suit this computer.".to_owned()
            },
            action: None,
        });
    }
    found
}

/// A ready-made set of video settings.
pub(crate) struct Profile {
    pub(crate) name: &'static str,
    pub(crate) tagline: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) apply: fn(&mut RecordingSettings),
    pub(crate) is_active: fn(&RecordingSettings) -> bool,
}

fn performance(s: &mut RecordingSettings) {
    s.quality = QualityMode::Standard;
    s.video_encoder = VideoEncoder::Gpu;
    s.encoder_preset = EncoderPreset::Fast;
}

fn balanced(s: &mut RecordingSettings) {
    s.quality = QualityMode::High;
    s.video_encoder = VideoEncoder::Gpu;
    s.encoder_preset = EncoderPreset::Balanced;
}

fn best_quality(s: &mut RecordingSettings) {
    s.quality = QualityMode::Custom;
    s.height = 1440;
    s.frames_per_second = 60;
    s.video_kbps = 25_000;
    s.video_encoder = VideoEncoder::Gpu;
    s.encoder_preset = EncoderPreset::Quality;
}

pub(crate) const PROFILES: [Profile; 3] = [
    Profile {
        name: "Performance",
        tagline: "720p \u{b7} 60 FPS",
        detail: "Smallest impact on your game. Small clips.",
        apply: performance,
        is_active: |s| {
            s.quality == QualityMode::Standard
                && s.video_encoder == VideoEncoder::Gpu
                && s.encoder_preset == EncoderPreset::Fast
        },
    },
    Profile {
        name: "Balanced",
        tagline: "1080p \u{b7} 60 FPS",
        detail: "Sharp and smooth, light on the processor.",
        apply: balanced,
        is_active: |s| {
            s.quality == QualityMode::High
                && s.video_encoder == VideoEncoder::Gpu
                && s.encoder_preset == EncoderPreset::Balanced
        },
    },
    Profile {
        name: "Best quality",
        tagline: "1440p \u{b7} 60 FPS \u{b7} 25M",
        detail: "For showing off. Big clips and a stronger graphics card.",
        apply: best_quality,
        is_active: |s| {
            s.quality == QualityMode::Custom
                && s.height == 1440
                && s.frames_per_second == 60
                && s.video_kbps == 25_000
                && s.video_encoder == VideoEncoder::Gpu
                && s.encoder_preset == EncoderPreset::Quality
        },
    },
];

/// How many saved preferences differ from a fresh install.
fn changed_count(settings: &RecordingSettings) -> usize {
    let (Ok(now), Ok(base)) = (
        serde_json::to_value(settings),
        serde_json::to_value(RecordingSettings::default()),
    ) else {
        return 0;
    };
    let (Some(now), Some(base)) = (now.as_object(), base.as_object()) else {
        return 0;
    };
    const SKIP: [&str; 10] = [
        "setup_completed",
        "game_rules",
        "disabled_steam_app_ids",
        "disabled_builtin_games",
        "game_integrations",
        "discord_presence",
        "capture_folder",
        "capture_target",
        "capture_target_name",
        "hotkeys",
    ];
    now.iter()
        .filter(|(key, value)| !SKIP.contains(&key.as_str()) && base.get(*key) != Some(value))
        .count()
}

fn stat_tile(ui: &mut egui::Ui, width: f32, label: &str, value: &str, note: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 92.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 14.0, CARD);
    ui.painter().rect_stroke(
        rect,
        14.0,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.top() + 18.0),
        egui::Align2::LEFT_CENTER,
        label.to_uppercase(),
        egui::FontId::new(10.5, kit::semibold()),
        FAINT,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.top() + 44.0),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::new(19.0, kit::semibold()),
        TEXT,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.top() + 70.0),
        egui::Align2::LEFT_CENTER,
        note,
        egui::FontId::proportional(12.0),
        MUTED,
    );
}

fn insight_card(ui: &mut egui::Ui, insight: &Insight) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                let (dot, _) = ui.allocate_exact_size(Vec2::new(10.0, 22.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), 4.5, insight.tone.color());
                ui.vertical(|ui| {
                    let room = (ui.available_width() - 190.0).max(160.0);
                    ui.set_max_width(room);
                    ui.label(
                        RichText::new(&insight.title)
                            .color(TEXT)
                            .size(14.5)
                            .family(kit::semibold()),
                    );
                    ui.label(RichText::new(&insight.body).color(MUTED).size(12.5));
                });
                if let Some((label, _)) = insight.action {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clicked =
                            kit::labelled_button(ui, kit::Icon::Gear, label, false, true).clicked();
                    });
                }
            });
        });
    clicked
}

fn profile_card(ui: &mut egui::Ui, width: f32, profile: &Profile, active: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 112.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, active, profile.name)
    });
    let hovered = response.hovered();
    ui.painter().rect_filled(
        rect,
        14.0,
        if active {
            Color32::from_rgba_unmultiplied(190, 255, 43, 18)
        } else if hovered {
            kit::CARD_HOVER
        } else {
            CARD
        },
    );
    ui.painter().rect_stroke(
        rect,
        14.0,
        Stroke::new(
            if active { 1.5_f32 } else { 1.0_f32 },
            if active { LIME } else { BORDER },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.top() + 22.0),
        egui::Align2::LEFT_CENTER,
        profile.name,
        egui::FontId::new(15.0, kit::semibold()),
        if active { LIME } else { TEXT },
    );
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.top() + 46.0),
        egui::Align2::LEFT_CENTER,
        profile.tagline,
        egui::FontId::proportional(12.5),
        TEXT,
    );
    let galley = ui.painter().layout(
        profile.detail.to_owned(),
        egui::FontId::proportional(12.0),
        MUTED,
        width - 32.0,
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 16.0, rect.top() + 62.0),
        galley,
        MUTED,
    );
    response.clicked() && !active
}

pub(crate) fn overview(
    ui: &mut egui::Ui,
    settings: &mut RecordingSettings,
    gpu: &GpuProbe,
    live: &Live,
) {
    let encoder = match (settings.video_encoder, gpu) {
        (VideoEncoder::Gpu, GpuProbe::Ready(_)) => "Graphics card".to_owned(),
        (VideoEncoder::Gpu, _) => "Processor (no GPU encoder)".to_owned(),
        (VideoEncoder::Cpu, _) => "Processor".to_owned(),
    };
    let gap = 12.0;
    let width = ((ui.available_width() - gap * 3.0) / 4.0).max(80.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        stat_tile(
            ui,
            width,
            "Picture",
            &format!("{}p", settings.height),
            &format!(
                "{} FPS \u{b7} {}",
                settings.frames_per_second,
                bitrate_label(settings.video_kbps)
            ),
        );
        stat_tile(
            ui,
            width,
            "Encoder",
            &encoder,
            &format!(
                "{} \u{b7} {}",
                settings.codec.label(),
                settings.encoder_preset.label()
            ),
        );
        stat_tile(
            ui,
            width,
            "Replay",
            &seconds_label(settings.replay_seconds + settings.post_roll_seconds),
            &if settings.post_roll_seconds > 0 {
                format!(
                    "+{} after hotkey",
                    seconds_label(settings.post_roll_seconds)
                )
            } else {
                "kept in memory".to_owned()
            },
        );
        stat_tile(
            ui,
            width,
            "A clip is",
            &format_bytes(settings.estimated_clip_bytes(settings.video_kbps)),
            "estimated",
        );
    });
    ui.add_space(26.0);
    section_heading(ui, "Suggestions");
    for insight in insights(settings, gpu, live) {
        if insight_card(ui, &insight)
            && let Some((_, apply)) = insight.action
        {
            apply(settings);
        }
        ui.add_space(8.0);
    }
    ui.add_space(18.0);
    section_heading(ui, "Profiles");
    let width = ((ui.available_width() - gap * 2.0) / 3.0).max(80.0);
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for profile in &PROFILES {
            if profile_card(ui, width, profile, (profile.is_active)(settings)) {
                chosen = Some(profile.apply);
            }
        }
    });
    if let Some(apply) = chosen {
        apply(settings);
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new("A profile sets quality, encoder and effort together. You can still change each one in Video.")
            .color(FAINT)
            .size(12.0),
    );
    ui.add_space(22.0);
    let changed = changed_count(settings);
    ui.label(
        RichText::new(if changed == 0 {
            "Every setting is at its default.".to_owned()
        } else {
            format!(
                "{changed} setting{} changed from the default. Changed settings show a Reset link.",
                if changed == 1 { "" } else { "s" }
            )
        })
        .color(MUTED)
        .size(12.5),
    );
    ui.add_space(20.0);
}

// ---------------------------------------------------------------------------------------------
// Video
// ---------------------------------------------------------------------------------------------

const FPS_STOPS: [u32; 6] = [24, 30, 60, 90, 120, 144];
const KBPS_STOPS: [u32; 12] = [
    1_000, 2_500, 5_000, 8_000, 10_000, 12_000, 16_000, 20_000, 25_000, 40_000, 60_000, 100_000,
];

pub(crate) fn video_rows(
    rows: &mut Rows,
    ui: &mut egui::Ui,
    s: &mut RecordingSettings,
    gpu: &GpuProbe,
) {
    rows.page(ui, "Video");
    rows.extra(ui, |ui| {
        card_plain(ui, "Recording quality", |ui| {
            ui.add_space(10.0);
            let gap = 10.0;
            let width = (ui.available_width() - gap * 3.0) / 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for mode in QualityMode::ALL {
                    let selected = s.quality == mode;
                    if quality_tile(ui, width, mode, selected, &tile_facts(mode, s)) {
                        s.quality = mode;
                    }
                }
            });
            clip_size_note(ui, s);
            if s.quality != QualityMode::Custom {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Choose Custom to set resolution, frame rate and bitrate yourself.",
                    )
                    .color(FAINT)
                    .size(12.0),
                );
            }
        });
    });
    rows.section(ui, "Custom picture");
    if s.quality == QualityMode::Custom || rows.searching() {
        rows.stepped(
            ui,
            s,
            Meta::new(
                "Resolution",
                "The height of the picture. Applies in Custom quality.",
                "1080p 1440p 4k 720p pixels height size",
            ),
            |s| &mut s.height,
            &HEIGHTS,
            height_label,
        );
        rows.stepped(
            ui,
            s,
            Meta::new(
                "Frame rate",
                "Frames per second. Higher is smoother and needs a faster encoder. Applies in Custom quality.",
                "fps 30 60 120 smooth lag stutter",
            ),
            |s| &mut s.frames_per_second,
            &FPS_STOPS,
            |fps| format!("{fps} FPS"),
        );
        rows.stepped(
            ui,
            s,
            Meta::new(
                "Video bitrate",
                "Data per second of video. Higher is sharper and makes bigger files. Applies in Custom quality.",
                "sharpness quality megabits mbps file size",
            ),
            |s| &mut s.video_kbps,
            &KBPS_STOPS,
            bitrate_label,
        );
    }
    rows.section(ui, "Encoder");
    let encoder_note = encoder_help(s.video_encoder, gpu);
    rows.choose(
        ui,
        s,
        Meta::new(
            "Video encoder",
            &encoder_note,
            "gpu cpu graphics card amd nvidia intel hardware software processor lag dropped frames",
        ),
        |s| &mut s.video_encoder,
        &[
            (
                VideoEncoder::Gpu,
                "GPU",
                true,
                "The graphics card encodes the video. The processor stays free for your game.",
            ),
            (
                VideoEncoder::Cpu,
                "CPU",
                true,
                "The processor encodes the video. Works everywhere, but takes processor time from your game.",
            ),
        ],
    );
    let codecs: Vec<(VideoCodec, String, bool, String)> = VideoCodec::ALL
        .iter()
        .map(|codec| {
            let reason = codec.unavailable_reason();
            (
                *codec,
                codec.label().to_owned(),
                reason.is_none(),
                reason.map_or_else(|| codec.label().to_owned(), str::to_owned),
            )
        })
        .collect();
    let codec_options: Vec<(VideoCodec, &str, bool, &str)> = codecs
        .iter()
        .map(|(codec, label, enabled, hint)| (*codec, label.as_str(), *enabled, hint.as_str()))
        .collect();
    let codec_note = if s.codec == VideoCodec::H265 && s.video_encoder == VideoEncoder::Cpu {
        "H.265 needs the graphics card to encode it. With the processor encoder ClipCraft records H.264."
    } else if s.codec == VideoCodec::H265 {
        "About a third to half the size at the same quality. Not every player or website opens H.265."
    } else {
        "H.264 plays everywhere. H.265 makes smaller files but needs the graphics card."
    };
    rows.choose(
        ui,
        s,
        Meta::new(
            "Codec",
            codec_note,
            "h264 h265 hevc avc compression smaller file size",
        ),
        |s| &mut s.codec,
        &codec_options,
    );
    let effort = s.encoder_preset.detail();
    rows.choose(
        ui,
        s,
        Meta::new(
            "Encoder effort",
            effort,
            "speed balanced quality preset fast slow cpu usage",
        ),
        |s| &mut s.encoder_preset,
        &[
            (
                EncoderPreset::Fast,
                "Fast",
                true,
                EncoderPreset::Fast.detail(),
            ),
            (
                EncoderPreset::Balanced,
                "Balanced",
                true,
                EncoderPreset::Balanced.detail(),
            ),
            (
                EncoderPreset::Quality,
                "Quality",
                true,
                EncoderPreset::Quality.detail(),
            ),
        ],
    );
    rows.section(ui, "Bitrate and keyframes");
    let mode_note = s.rate_control.detail();
    rows.choose(
        ui,
        s,
        Meta::new("Bitrate mode", mode_note, "variable constant vbr cbr"),
        |s| &mut s.rate_control,
        &[
            (
                RateControl::Variable,
                "Variable",
                true,
                RateControl::Variable.detail(),
            ),
            (
                RateControl::Constant,
                "Constant",
                true,
                RateControl::Constant.detail(),
            ),
        ],
    );
    rows.range(
        ui,
        s,
        Meta::new(
            "Keyframe interval",
            "Seconds between keyframes. Shorter clips scrub faster; longer ones are smaller.",
            "gop seek scrub",
        ),
        |s| &mut s.keyframe_seconds,
        1..=10,
        1,
        |seconds| format!("{seconds} s"),
    );
    rows.section(ui, "File");
    let formats: Vec<(Container, String, bool, String)> = Container::ALL
        .iter()
        .map(|container| {
            let reason = container.unavailable_reason();
            (
                *container,
                container.label().to_owned(),
                reason.is_none(),
                reason.map_or_else(|| container.detail().to_owned(), str::to_owned),
            )
        })
        .collect();
    let format_options: Vec<(Container, &str, bool, &str)> = formats
        .iter()
        .map(|(c, label, enabled, hint)| (*c, label.as_str(), *enabled, hint.as_str()))
        .collect();
    let format_note = s.container.detail();
    rows.choose(
        ui,
        s,
        Meta::new("File format", format_note, "container mp4 mkv webm"),
        |s| &mut s.container,
        &format_options,
    );
    rows.finish(ui);
}

pub fn video_tab(ui: &mut egui::Ui, settings: &mut RecordingSettings, gpu: &GpuProbe) {
    let mut rows = Rows::new("");
    video_rows(&mut rows, ui, settings, gpu);
}

// ---------------------------------------------------------------------------------------------
// Replay
// ---------------------------------------------------------------------------------------------

const REPLAY_STOPS: [u32; 12] = [5, 10, 15, 30, 45, 60, 90, 120, 180, 300, 450, 600];

pub(crate) fn seconds_label(seconds: u32) -> String {
    match seconds {
        0 => "Off".to_owned(),
        s if s < 60 => format!("{s} s"),
        s if s.is_multiple_of(60) => format!("{} min", s / 60),
        s => format!("{}:{:02}", s / 60, s % 60),
    }
}

pub(crate) fn replay_rows(rows: &mut Rows, ui: &mut egui::Ui, s: &mut RecordingSettings) {
    rows.page(ui, "Replay");
    rows.section(ui, "Instant replay");
    rows.stepped(
        ui,
        s,
        Meta::new(
            "Replay length",
            "How much play before the hotkey is kept, in memory, until you save a clip.",
            "instant replay buffer seconds minutes how far back",
        ),
        |s| &mut s.replay_seconds,
        &REPLAY_STOPS,
        seconds_label,
    );
    rows.range(
        ui,
        s,
        Meta::new(
            "Keep recording after the hotkey",
            "Adds the moments after you press the key, such as the reaction. The clip is saved once they have been recorded.",
            "post roll after seconds reaction",
        ),
        |s| &mut s.post_roll_seconds,
        0..=30,
        1,
        seconds_label,
    );
    rows.section(ui, "When a clip is saved");
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Show a notification",
            "A card appears in the corner of the screen, even over a game, with the clip's picture and buttons to play it, find it or delete it.",
            "popup toast notice saved clip corner overlay",
        ),
        |s| &mut s.show_save_notification,
    );
    rows.finish(ui);
}

pub fn replay_tab(ui: &mut egui::Ui, settings: &mut RecordingSettings) {
    // Say how the pieces fit together, since what is recorded is chosen on another page.
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::symmetric(20, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("How clipping works")
                    .color(TEXT)
                    .size(15.0)
                    .family(kit::semibold()),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "1. Capture (its own page) picks what is recorded: a screen, a window, or whichever game is running.
                     2. While clipping is on, ClipCraft keeps only the last few moments in memory. Nothing is written to disk yet.
                     3. Press the save hotkey and that stretch is saved as a clip. The settings below set how long it is.",
                )
                .color(MUTED)
                .size(13.0),
            );
        });
    ui.add_space(14.0);
    let mut rows = Rows::new("");
    replay_rows(&mut rows, ui, settings);
    section_heading(ui, "What a clip will be");
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::symmetric(20, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let total = settings.replay_seconds + settings.post_roll_seconds;
            info_row(ui, "Length", &seconds_label(total), false);
            info_row(
                ui,
                "Picture",
                &format!(
                    "{}p \u{b7} {} FPS \u{b7} {}",
                    settings.height,
                    settings.frames_per_second,
                    bitrate_label(settings.video_kbps)
                ),
                false,
            );
            info_row(
                ui,
                "About",
                &format_bytes(settings.estimated_clip_bytes(settings.video_kbps)),
                true,
            );
        });
    ui.add_space(8.0);
    ui.label(
        RichText::new("Sizes are estimated from the bitrate. Change quality in Video.")
            .color(FAINT)
            .size(12.0),
    );
}

// ---------------------------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------------------------

fn percent(value: u32) -> String {
    format!("{value}%")
}

/// A device menu that fits in the control column of a row.
fn device_menu(
    ui: &mut egui::Ui,
    name: &str,
    current: &str,
    options: &[(String, String)],
) -> Option<usize> {
    let mut chosen = None;
    let shown: String = if current.chars().count() > 26 {
        current
            .chars()
            .take(25)
            .chain(std::iter::once('\u{2026}'))
            .collect()
    } else {
        current.to_owned()
    };
    kit::pill_menu(ui, &shown, name, |ui| {
        ui.set_min_width(220.0);
        for (index, (_, text)) in options.iter().enumerate() {
            if ui
                .add(egui::Button::new(text.as_str()).frame(false))
                .clicked()
            {
                chosen = Some(index);
                ui.close();
            }
        }
    });
    chosen
}

pub(crate) fn audio_rows(
    rows: &mut Rows,
    ui: &mut egui::Ui,
    s: &mut RecordingSettings,
    state: &mut SettingsState,
) {
    if state.devices.is_none() {
        refresh_devices(state);
    }
    let devices = state.devices.clone().unwrap_or_default();
    rows.page(ui, "Audio");
    rows.section(ui, "PC audio");
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Record game and system sound",
            "Everything you hear from the output device below.",
            "game sound loopback speakers pc audio",
        ),
        |s| &mut s.capture_game_audio,
    );
    {
        let mut options = vec![(String::new(), "System default".to_owned())];
        options.extend(state.outputs.iter().cloned());
        let current = options
            .iter()
            .find(|(name, _)| *name == s.output_device)
            .map_or_else(
                || {
                    if s.output_device.is_empty() {
                        "System default".to_owned()
                    } else {
                        s.output_device.clone()
                    }
                },
                |(_, shown)| shown.clone(),
            );
        let changed = !s.output_device.is_empty();
        let mut pick = None;
        let reset = rows.custom(
            ui,
            Meta::new(
                "Output device to record",
                "Which speakers or headphones to record the sound of.",
                "speakers headphones output device",
            ),
            changed,
            250.0,
            |ui| pick = device_menu(ui, "Output device", &current, &options),
        );
        if let Some(index) = pick {
            s.output_device = options[index].0.clone();
        }
        if reset {
            s.output_device.clear();
        }
    }
    rows.range(
        ui,
        s,
        Meta::new(
            "Game volume",
            "How loud the game is in the clip.",
            "volume level loud quiet",
        ),
        |s| &mut s.game_audio_percent,
        0..=100,
        1,
        percent,
    );
    rows.section(ui, "Microphone");
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Record microphone",
            "Your voice goes into the clip.",
            "mic voice input commentary",
        ),
        |s| &mut s.capture_microphone,
    );
    {
        let mut options = vec![(String::new(), "System default".to_owned())];
        options.extend(devices.inputs.iter().map(|n| (n.clone(), n.clone())));
        let current = if s.microphone_device.is_empty() {
            "System default".to_owned()
        } else {
            s.microphone_device.clone()
        };
        let changed = !s.microphone_device.is_empty();
        let mut pick = None;
        let mut refresh = false;
        let reset = rows.custom(
            ui,
            Meta::new(
                "Microphone device",
                "Which microphone to record.",
                "mic input device choose",
            ),
            changed,
            330.0,
            |ui| {
                if kit::labelled_button(ui, kit::Icon::Loop, "Refresh", false, true).clicked() {
                    refresh = true;
                }
                pick = device_menu(ui, "Microphone", &current, &options);
            },
        );
        if let Some(index) = pick {
            s.microphone_device = options[index].0.clone();
        }
        if reset {
            s.microphone_device.clear();
        }
        if refresh {
            refresh_devices(state);
        }
    }
    rows.range(
        ui,
        s,
        Meta::new(
            "Microphone volume",
            "How loud your voice is in the clip.",
            "mic volume level loud quiet",
        ),
        |s| &mut s.microphone_percent,
        0..=100,
        1,
        percent,
    );
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Noise suppression",
            "Reduce background noise in your recorded voice, offline. Preserves stereo unless mono is selected. Applies when clipping restarts; Mic Test updates live.",
            "microphone mic voice noise reduction denoise rnnoise fan keyboard hiss",
        ),
        |s| &mut s.microphone_noise_suppression,
    );
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Microphone is mono",
            "Mix the microphone's left and right channels into the same sound on both sides, including with noise suppression.",
            "mic mono one ear left right",
        ),
        |s| &mut s.microphone_mono,
    );
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Push to talk",
            "Your microphone is only recorded while you hold the push-to-talk key. Set the key in Hotkeys.",
            "microphone key hold ptt",
        ),
        |s| &mut s.push_to_talk,
    );
    {
        let snapshot = s.clone();
        // Rows reserve 34 px for the control. Status belongs in the wrapping left-hand help,
        // rather than adding vertical content to the right-to-left button/meter layout.
        let help = if let Some(error) = &state.mic_error {
            error.clone()
        } else if state.mic.as_ref().is_some_and(MicTest::ready) {
            "Listening after noise suppression, volume and mono. Changes apply live. Stops after 60 seconds. Use headphones to prevent feedback.".to_owned()
        } else if state.mic.is_some() {
            "Opening microphone and playback device… Use headphones to prevent feedback.".to_owned()
        } else {
            "Hear yourself after noise suppression, volume and mono settings. Use headphones to prevent feedback.".to_owned()
        };
        rows.custom(
            ui,
            Meta::new(
                "Test your microphone",
                &help,
                "mic level meter check input listen playback monitor test",
            ),
            false,
            330.0,
            |ui| mic_test(ui, &snapshot, state),
        );
    }
    rows.section(ui, "Sound effects");
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Play sound effects",
            "A short sound when you start or stop clipping, save a clip, take a screenshot, or switch your microphone.",
            "cues beeps sounds notification",
        ),
        |s| &mut s.sound_effects,
    );
    rows.range(
        ui,
        s,
        Meta::new(
            "Effects volume",
            "How loud the sound effects are.",
            "cues volume level",
        ),
        |s| &mut s.sound_effects_percent,
        0..=100,
        1,
        percent,
    );
    {
        let enabled = s.sound_effects;
        let volume = s.sound_effects_percent;
        rows.custom(
            ui,
            Meta::new(
                "Hear a sample",
                "Plays the sound you get when a clip is saved.",
                "test sound effect",
            ),
            false,
            150.0,
            |ui| {
                if kit::labelled_button(ui, kit::Icon::Speaker, "Play sample", false, enabled)
                    .clicked()
                {
                    clipcraft_audio_out::play_cue(
                        clipcraft_audio_out::Cue::ClipSaved,
                        volume.min(100) as u8,
                    );
                }
            },
        );
    }
    rows.section(ui, "Tracks and quality");
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Separate audio tracks",
            "Save game sound and microphone as their own tracks so you can edit them apart. Without this they are mixed into one.",
            "game mic mixed tracks edit",
        ),
        |s| &mut s.separate_audio_tracks,
    );
    rows.stepped(
        ui,
        s,
        Meta::new(
            "Audio bitrate",
            "AAC audio, kilobits per second. 256 and above is transparent for most listeners.",
            "aac kbps sound quality",
        ),
        |s| &mut s.audio_kbps,
        &AUDIO_KBPS,
        |kbps| format!("{kbps} kbps"),
    );
    rows.choose(
        ui,
        s,
        Meta::new(
            "Sample rate",
            "48 kHz is the video standard.",
            "khz hertz 44.1 48",
        ),
        |s| &mut s.audio_sample_rate,
        &[
            (44_100, "44.1 kHz", true, "Sample rate"),
            (48_000, "48 kHz", true, "Sample rate"),
        ],
    );
    rows.choose(
        ui,
        s,
        Meta::new(
            "Channels",
            "Stereo keeps left and right apart.",
            "stereo mono",
        ),
        |s| &mut s.audio_channels,
        &[(2, "Stereo", true, "Stereo"), (1, "Mono", true, "Mono")],
    );
    rows.finish(ui);
}

pub fn audio_tab(ui: &mut egui::Ui, settings: &mut RecordingSettings, state: &mut SettingsState) {
    let mut rows = Rows::new("");
    audio_rows(&mut rows, ui, settings, state);
    if settings.push_to_talk && settings.hotkeys.get(HotkeyAction::PushToTalk).is_empty() {
        ui.label(
            RichText::new("No push-to-talk key is set yet. Choose one in Hotkeys.")
                .color(DANGER)
                .size(12.5),
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------------------------

fn shape_index(s: &RecordingSettings) -> usize {
    match (s.shape, s.framing) {
        (OutputShape::Source, _) => 0,
        (_, Framing::Letterbox) => 2,
        (_, Framing::Crop) => 3,
        _ => 1,
    }
}

pub(crate) fn capture_rows(
    rows: &mut Rows,
    ui: &mut egui::Ui,
    s: &mut RecordingSettings,
    env: &mut Env<'_>,
) {
    rows.page(ui, "Capture");
    rows.section(ui, "What to record");
    rows.choose(
        ui,
        s,
        Meta::new(
            "Capture",
            "Record a game window or a whole monitor.",
            "source game desktop monitor screen window display",
        ),
        |s| &mut s.source,
        &[
            (
                CaptureSource::Game,
                CaptureSource::Game.label(),
                true,
                CaptureSource::Game.detail(),
            ),
            (
                CaptureSource::Desktop,
                CaptureSource::Desktop.label(),
                true,
                CaptureSource::Desktop.detail(),
            ),
        ],
    );
    if !rows.searching() {
        let want_window = s.source == CaptureSource::Game;
        let kind = if want_window {
            clipcraft_capture::TargetKind::Window
        } else {
            clipcraft_capture::TargetKind::Display
        };
        let choices: Vec<&clipcraft_capture::CaptureTarget> = env
            .targets
            .iter()
            .filter(|target| target.kind == kind)
            .collect();
        let current = match choices.iter().find(|target| target.id == s.capture_target) {
            Some(target) => target.name.clone(),
            None if !s.capture_target.is_empty() => {
                format!("{} (not open)", s.capture_target_name)
            }
            None if want_window => "Choose a window".to_owned(),
            None => "Main monitor".to_owned(),
        };
        let mut options = vec![(
            String::new(),
            if want_window {
                "None chosen".to_owned()
            } else {
                "Main monitor".to_owned()
            },
        )];
        options.extend(choices.iter().map(|target| {
            (
                target.id.clone(),
                format!("{}  ({}\u{d7}{})", target.name, target.width, target.height),
            )
        }));
        let mut pick = None;
        let mut refresh = false;
        let title = if want_window {
            "Game window"
        } else {
            "Monitor"
        };
        let changed = !s.capture_target.is_empty();
        let reset = rows.custom(
            ui,
            Meta::new(
                title,
                if want_window {
                    "Which window to record."
                } else {
                    "Which monitor to record."
                },
                "target display screen window choose",
            ),
            changed,
            330.0,
            |ui| {
                if kit::labelled_button(ui, kit::Icon::Loop, "Refresh", false, true).clicked() {
                    refresh = true;
                }
                pick = device_menu(ui, title, &current, &options);
            },
        );
        if let Some(index) = pick {
            match index.checked_sub(1).and_then(|at| choices.get(at)) {
                Some(target) => {
                    s.capture_target = target.id.clone();
                    s.capture_target_name = target.name.clone();
                }
                None => {
                    s.capture_target.clear();
                    s.capture_target_name.clear();
                }
            }
        }
        if reset {
            s.capture_target.clear();
            s.capture_target_name.clear();
        }
        if refresh {
            *env.refresh_targets = true;
        }
        if want_window && choices.is_empty() {
            ui.label(
                RichText::new("No windows were found. Open the game, then press Refresh.")
                    .color(FAINT)
                    .size(12.5),
            );
        }
    }
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Start clipping when ClipCraft opens",
            "Starts filling the replay buffer as soon as the app starts.",
            "record on launch automatic arm replay buffer startup",
        ),
        |s| &mut s.record_on_launch,
    );
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Show the mouse cursor",
            "Draw the pointer in your clips.",
            "mouse pointer cursor hide show",
        ),
        |s| &mut s.capture_cursor,
    );
    let window_note = if env.capabilities.windows {
        "Record the window that has the focus (your game) even when you picked a whole monitor, and never the monitor around it. Try this if a game records black."
    } else {
        "Window capture is not available on this computer."
    };
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Force window capture",
            window_note,
            "game window fullscreen borderless black screen",
        ),
        |s| &mut s.force_window_capture,
    );
    let hdr_note = if env.capabilities.hdr {
        "Only for an HDR monitor. Converts HDR to standard colours so clips are not washed out when shared. It uses more processor, so leave it off unless your clips look washed out."
    } else {
        "This computer's capture does not report HDR, so this has no effect here."
    };
    rows.toggle(
        ui,
        s,
        Meta::new(
            "HDR compatibility",
            hdr_note,
            "tone map colours washed out sdr hdr",
        ),
        |s| &mut s.hdr_compatibility,
    );
    rows.section(ui, "Picture shape");
    {
        let current = shape_index(s);
        let changed = current != 0;
        let mut pick = None;
        let options = [
            ("Game", "Clips keep the game's own shape, 4:3 stays 4:3."),
            (
                "Stretch",
                "Fills a widescreen frame. A 4:3 game looks wider.",
            ),
            ("Bars", "Fits 16:9 with black bars, keeping the shape."),
            ("Crop", "Crops to 16:9, trimming the top and bottom."),
        ];
        let reset = rows.custom(
            ui,
            Meta::new(
                "Video aspect ratio",
                options[current].1,
                "shape crop stretch letterbox vertical portrait black bars 16:9 4:3",
            ),
            changed,
            330.0,
            |ui| {
                let stacked = ui.layout().main_dir() == egui::Direction::LeftToRight;
                let order: Vec<usize> = if stacked {
                    (0..4).collect()
                } else {
                    (0..4).rev().collect()
                };
                for index in order {
                    if choice(
                        ui,
                        options[index].0,
                        current == index,
                        true,
                        options[index].1,
                    ) {
                        pick = Some(index);
                    }
                }
            },
        );
        if let Some(index) = pick {
            match index {
                0 => {
                    s.shape = OutputShape::Source;
                    s.framing = Framing::Original;
                }
                1 => {
                    s.shape = OutputShape::Wide16x9;
                    s.framing = Framing::Stretch;
                }
                2 => {
                    s.shape = OutputShape::Wide16x9;
                    s.framing = Framing::Letterbox;
                }
                _ => {
                    s.shape = OutputShape::Wide16x9;
                    s.framing = Framing::Crop;
                }
            }
        }
        if reset {
            s.shape = OutputShape::Source;
            s.framing = Framing::Original;
        }
    }
    rows.finish(ui);
}

// ---------------------------------------------------------------------------------------------
// Storage limit and General switches
// ---------------------------------------------------------------------------------------------

pub(crate) fn storage_rows(rows: &mut Rows, ui: &mut egui::Ui, s: &mut RecordingSettings) {
    rows.page(ui, "Storage");
    rows.section(ui, "Storage limit");
    rows.stepped(
        ui,
        s,
        Meta::new(
            "Most space clips may use",
            "ClipCraft stops saving new clips, or deletes the oldest, when this is reached.",
            "storage limit maximum disk space gigabytes full",
        ),
        |s| &mut s.storage_limit_gb,
        &[0, 10, 25, 50, 100, 250, 500, 1000, 2000],
        |gigabytes| match gigabytes {
            0 => "Unlimited".to_owned(),
            1000 => "1 TB".to_owned(),
            2000 => "2 TB".to_owned(),
            other => format!("{other} GB"),
        },
    );
    if s.storage_limit_gb > 0 || rows.searching() {
        rows.choose(
            ui,
            s,
            Meta::new(
                "When the limit is reached",
                "Delete the oldest clips to keep recording, or stop saving new ones. Nothing is deleted unless you choose that.",
                "delete oldest full stop saving cleanup",
            ),
            |s| &mut s.delete_oldest_when_full,
            &[
                (true, "Delete the oldest", true, "Keeps recording by making room."),
                (false, "Stop saving", true, "Nothing is ever deleted for you."),
            ],
        );
    }
    rows.finish(ui);
}

pub(crate) fn app_rows(rows: &mut Rows, ui: &mut egui::Ui, s: &mut RecordingSettings) {
    rows.page(ui, "General");
    rows.section(ui, "Startup and window");
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Launch ClipCraft when I sign in",
            "Starts quietly in the background so clips are ready when you play.",
            "startup boot windows login autostart",
        ),
        |s| &mut s.launch_at_startup,
    );
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Start minimized",
            "Open without showing the window.",
            "startup background hidden",
        ),
        |s| &mut s.start_minimized,
    );
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Keep running in the tray",
            "Closing the window minimizes ClipCraft and keeps it running, so recording continues. Open or quit it from the tray icon.",
            "system tray close minimize background notification area",
        ),
        |s| &mut s.close_to_tray,
    );
    rows.toggle(
        ui,
        s,
        Meta::new(
            "Check for updates",
            "Once at start-up, ClipCraft asks GitHub which version is newest and offers it if yours is older. Nothing about you or your clips is sent. Off, ClipCraft never goes online.",
            "update upgrade new version network internet online automatic",
        ),
        |s| &mut s.check_for_updates,
    );
    rows.finish(ui);
}

/// Every row of every page, drawn only where it matches the search. Pages that need the computer's
/// devices skip them when there is no state to ask.
pub(crate) fn search_results(
    ui: &mut egui::Ui,
    query: &str,
    s: &mut RecordingSettings,
    env: &mut Env<'_>,
    state: &mut SettingsState,
) -> usize {
    let mut rows = Rows::new(query);
    video_rows(&mut rows, ui, s, env.gpu);
    replay_rows(&mut rows, ui, s);
    audio_rows(&mut rows, ui, s, state);
    capture_rows(&mut rows, ui, s, env);
    storage_rows(&mut rows, ui, s);
    app_rows(&mut rows, ui, s);
    rows.shown
}
