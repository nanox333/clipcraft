//! The sound mixer's application rows: record the game, Discord, a music player or any other
//! program on a track of its own, with a level for each. Part of Settings > Audio.

use super::*;
use crate::rows::{Meta, Rows};
use clipcraft_engine::{AppAudioRow, AppAudioTarget, MAX_AUDIO_APPS};

/// What the Add menu offers, from the programs running now: unique file names, the clipcraft app
/// and the system's own helpers left out, shortest list first.
fn candidates(processes: &[clipcraft_platform::RunningProcess]) -> Vec<String> {
    let mut names: Vec<String> = processes
        .iter()
        .map(|process| process.executable.clone())
        .filter(|name| !name.is_empty())
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            !lower.starts_with("clipcraft")
                && !matches!(
                    lower.as_str(),
                    "system"
                        | "idle"
                        | "registry"
                        | "svchost.exe"
                        | "csrss.exe"
                        | "dwm.exe"
                        | "explorer.exe"
                        | "conhost.exe"
                        | "services.exe"
                        | "lsass.exe"
                )
        })
        .collect();
    names.sort_by_key(|name| name.to_ascii_lowercase());
    names.dedup_by_key(|name| name.to_ascii_lowercase());
    names.truncate(60);
    names
}

/// The section of Settings > Audio for application rows.
pub(crate) fn mixer_rows(
    rows: &mut Rows,
    ui: &mut egui::Ui,
    s: &mut RecordingSettings,
    state: &mut SettingsState,
) {
    let problem = clipcraft_platform::app_audio_problem();
    mixer_rows_with(rows, ui, s, state, problem.as_deref());
}

/// As [`mixer_rows`], saying whether this system can record one application's sound.
pub(crate) fn mixer_rows_with(
    rows: &mut Rows,
    ui: &mut egui::Ui,
    s: &mut RecordingSettings,
    state: &mut SettingsState,
    problem: Option<&str>,
) {
    let available = problem.is_none();
    rows.section(ui, "Applications");
    rows.extra(ui, |ui| {
        let note = problem.unwrap_or(
            "Record the game, Discord, a music player or any program on a track of its own, so you can edit them apart. Without this, everything you hear is one sound. The program is picked up when it starts playing, even if it was opened later.",
        );
        ui.label(RichText::new(note).color(MUTED).size(12.5));
        ui.add_space(6.0);
    });
    // The system sound includes these programs too: they would be recorded twice.
    if available && s.capture_game_audio && s.audio_apps.iter().any(|row| row.own_track) {
        rows.extra(ui, |ui| {
            ui.label(
                RichText::new(
                    "\u{26a0} \u{201c}Record game and system sound\u{201d} above also contains these programs, so they are in two places. Turn it off to keep them only on their own tracks.",
                )
                .color(kit::DANGER)
                .size(12.5),
            );
            ui.add_space(6.0);
        });
    }
    let mut remove = None;
    let live = state.live_audio.clone();
    for (index, row) in s.audio_apps.iter_mut().enumerate() {
        let status = live.iter().find(|status| status.name == row.name);
        let help = match status {
            Some(status) if status.lost => status
                .error
                .clone()
                .unwrap_or_else(|| "Waiting for it to play sound".to_owned()),
            Some(status) if status.muted => "Muted".to_owned(),
            Some(_) => "Recording".to_owned(),
            None => match &row.target {
                AppAudioTarget::Game => "The game being recorded".to_owned(),
                AppAudioTarget::Executable(name) => name.clone(),
            },
        };
        let help = format!(
            "{help}. {}",
            if row.own_track {
                "On its own track."
            } else {
                "Mixed into the main sound."
            }
        );
        let level = status.map_or(0.0, |status| status.level);
        let name = row.name.clone();
        let search = format!("{name} application program audio mixer track discord music game");
        rows.custom(ui, Meta::new(&name, &help, &search), false, 330.0, |ui| {
            if kit::labelled_button(ui, kit::Icon::Trash, "Remove", false, true)
                .on_hover_text("Stop recording this application on its own")
                .clicked()
            {
                remove = Some(index);
            }
            let own = ui
                .checkbox(&mut row.own_track, "Own track")
                .on_hover_text("Keep it on a track of its own in the clip");
            own.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Checkbox,
                    true,
                    format!("{name}: own track"),
                )
            });
            let mut percent = row.percent;
            let slider = ui.add(
                egui::Slider::new(&mut percent, 0..=100)
                    .suffix("%")
                    .show_value(true),
            );
            slider.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, format!("{name} volume"))
            });
            row.percent = percent;
            // A thin meter of what is coming in right now.
            let (rect, _) = ui.allocate_exact_size(egui::vec2(46.0, 6.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, kit::CARD);
            let filled = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(rect.width() * level.sqrt().clamp(0.0, 1.0), rect.height()),
            );
            ui.painter().rect_filled(filled, 3.0, kit::LIME);
        });
    }
    if let Some(index) = remove {
        s.audio_apps.remove(index);
    }
    if s.audio_apps.len() < MAX_AUDIO_APPS {
        let has_game = s
            .audio_apps
            .iter()
            .any(|row| row.target == AppAudioTarget::Game);
        let taken: Vec<String> = s
            .audio_apps
            .iter()
            .filter_map(|row| match &row.target {
                AppAudioTarget::Executable(name) => Some(name.to_ascii_lowercase()),
                AppAudioTarget::Game => None,
            })
            .collect();
        let mut add = None;
        rows.custom(
            ui,
            Meta::new(
                "Add an application",
                if available {
                    "Pick the game being recorded, or a program that is running now."
                } else {
                    "Not available on this system yet."
                },
                "add application program discord music game mixer",
            ),
            false,
            330.0,
            |ui| {
                ui.add_enabled_ui(available, |ui| {
                    kit::pill_menu(ui, "Add application", "Add application", |ui| {
                        ui.set_min_width(240.0);
                        if !has_game
                            && ui
                                .add(egui::Button::new("The game being recorded").frame(false))
                                .clicked()
                        {
                            add = Some(AppAudioRow::for_game());
                            ui.close();
                        }
                        let found = state.mixer_candidates.get_or_insert_with(|| {
                            candidates(&clipcraft_platform::running_processes(false))
                        });
                        for name in found.iter() {
                            if taken.contains(&name.to_ascii_lowercase()) {
                                continue;
                            }
                            if ui
                                .add(egui::Button::new(name.as_str()).frame(false))
                                .clicked()
                            {
                                add = Some(AppAudioRow::for_program(name));
                                ui.close();
                            }
                        }
                    });
                });
            },
        );
        if let Some(row) = add {
            s.audio_apps.push(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_platform::RunningProcess;

    #[test]
    fn the_add_menu_lists_each_program_once_without_clipcraft_or_system_helpers() {
        let process = |name: &str| RunningProcess {
            process_id: 1,
            executable: name.to_owned(),
            steam_app_id: None,
            parent_id: None,
        };
        let list = candidates(&[
            process("Discord.exe"),
            process("discord.exe"),
            process("svchost.exe"),
            process("clipcraft.exe"),
            process("Spotify.exe"),
            process(""),
        ]);
        assert_eq!(list, ["Discord.exe", "Spotify.exe"]);
    }
}
