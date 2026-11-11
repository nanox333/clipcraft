//! The Storage tab: where clips go, how much space they use and the size limit.

use super::*;

pub(crate) fn request_usage(
    state: &mut SettingsState,
    context: &egui::Context,
    folder: PathBuf,
    force: bool,
) {
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
pub(crate) fn usage_bar(ui: &mut egui::Ui, fraction: f32, warn: bool) {
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
    let mut rows = crate::rows::Rows::new("");
    pages::storage_rows(&mut rows, ui, settings);
}
