//! The Games tab: recognized games, the custom game form and the process picker.

use super::*;

/// One known game in the list: its name and a switch for whether it is recognised. True when
/// the row is clicked.
pub(crate) fn game_row(ui: &mut egui::Ui, name: &str, included: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, included, name)
    });
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_white_alpha(8));
    }
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::new(14.0, kit::medium()),
        if included { TEXT } else { MUTED },
    );
    let switch = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 36.0, rect.center().y),
        Vec2::new(40.0, 22.0),
    );
    paint_switch(ui.painter(), switch, included);
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(rect.left() + 16.0, rect.bottom() - 1.0),
            Vec2::new(rect.width() - 32.0, 1.0),
        ),
        0.0,
        BORDER,
    );
    response.clicked()
}

/// Process rules that are allowed to start the replay buffer.
pub fn games_tab(
    ui: &mut egui::Ui,
    settings: &mut RecordingSettings,
    state: &mut SettingsState,
    game_data: &mut crate::game_data::State,
) -> bool {
    let mut refresh_processes = false;
    card(ui, "Recording behavior", |ui| {
        toggle_row(
            ui,
            "Switch recording automatically",
            "Turn off to choose a screen or window manually in Capture.",
            &mut settings.game_only_capture,
            true,
        );
        if settings.game_only_capture {
            for (mode, label, help) in [
                (
                    DesktopCaptureMode::Never,
                    "Games only",
                    "Keep replay stopped until a recognized game opens. Stop when the game closes.",
                ),
                (
                    DesktopCaptureMode::AfterGame,
                    "Games, then desktop briefly",
                    "When a game closes or crashes, record the desktop for the duration below. Reopening a game switches straight back to game clipping.",
                ),
                (
                    DesktopCaptureMode::Always,
                    "Desktop and games",
                    "Keep a desktop replay running whenever no recognized game is open, including at startup. Switch to each game when it opens and back to desktop when it closes.",
                ),
            ] {
                if radio_row(ui, label, help, settings.desktop_capture_mode == mode) {
                    settings.desktop_capture_mode = mode;
                    settings.record_desktop_after_game = false;
                }
            }
            if settings.desktop_capture_mode == DesktopCaptureMode::AfterGame {
                setting_row(
                    ui,
                    "Desktop recording duration",
                    "A fresh timer starts each time a game closes. A new game cancels the timer and takes over recording.",
                    true,
                    |ui| {
                        for (seconds, label) in [
                            (30, "30 sec"),
                            (60, "1 min"),
                            (120, "2 min"),
                            (300, "5 min"),
                            (600, "10 min"),
                        ] {
                            if choice(
                                ui,
                                label,
                                settings.desktop_after_game_seconds == seconds,
                                true,
                                "Desktop recording duration",
                            ) {
                                settings.desktop_after_game_seconds = seconds;
                            }
                        }
                        ui.label("Custom:");
                        let response = ui.add(
                            egui::DragValue::new(&mut settings.desktop_after_game_seconds)
                                .range(15..=3600)
                                .suffix(" sec"),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::DragValue,
                                true,
                                "Desktop recording duration in seconds",
                            )
                        });
                    },
                );
            }
            toggle_row(
                ui,
                "Record the screen when a game's window can't be found",
                "Some games draw in a window ClipCraft cannot pick out. Record the screen instead and switch to the game's window when one appears. The screen can include other apps.",
                &mut settings.record_screen_when_game_window_missing,
                true,
            );
            if settings.desktop_capture_mode != DesktopCaptureMode::Never {
                ui.add_space(8.0);
                ui.label(RichText::new("Desktop capture can include other apps. These modes keep a bounded replay buffer. Nothing is saved when a game closes or the desktop timer ends; clips are saved only when you press the save shortcut. Stop pauses automatic recording until you press Start again.").color(MUTED).size(12.5));
            }
        }
    });

    card(ui, "Known games", |ui| {
        toggle_row(
            ui,
            "Recognize known games automatically",
            "Built-in game detection works without adding games manually. Custom rules take priority.",
            &mut settings.detect_built_in_games,
            true,
        );
        ui.add_space(4.0);
        let response = search_box(ui, &mut state.builtin_game_search, "Search known games");
        kit::name_text_box(&response, "Search known games", &state.builtin_game_search);
        ui.add_space(12.0);
        let query = state.builtin_game_search.trim().to_lowercase();
        let mut change = None;
        let mut found = 0;
        egui::Frame::new()
            .fill(Color32::from_rgb(22, 22, 26))
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(egui::CornerRadius::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("known-games-list")
                    .max_height(300.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for game in clipcraft_engine::built_in_games() {
                            if !game.name.to_lowercase().contains(&query)
                                && !game.id.contains(&query)
                            {
                                continue;
                            }
                            found += 1;
                            let included = !settings
                                .disabled_builtin_games
                                .iter()
                                .any(|id| id == game.id);
                            if game_row(ui, game.name, included) {
                                change = Some((game.id.to_owned(), !included));
                            }
                        }
                    });
            });
        ui.add_space(10.0);
        if found == 0 {
            ui.label(
                RichText::new("No known games match. You can add a custom game below.")
                    .color(FAINT)
                    .size(12.5),
            );
        }
        if let Some((id, enabled)) = change {
            state.game_preset_error = set_game_presets(settings, vec![id], enabled)
                .err()
                .map(|error| error.to_string());
        }
        if let Some(error) = &state.game_preset_error {
            ui.label(RichText::new(error).color(DANGER).size(12.5));
        }
        ui.add_space(5.0);
        ui.label(
            RichText::new(if settings.detect_built_in_games {
                format!(
                    "{} built-in games. Uncheck a game to exclude it from automatic recognition.",
                    clipcraft_engine::built_in_games().len()
                )
            } else {
                "Built-in recognition is off. Custom rules still apply.".to_owned()
            })
            .color(MUTED)
            .size(12.5),
        );
    });

    card(ui, "Installed Steam games", |ui| {
        toggle_row(
            ui,
            "Recognize installed Steam games",
            "Read the game names and IDs in your local Steam libraries. No account or download is needed. The automatic-recognition switch above must also be on.",
            &mut settings.detect_installed_steam_games,
            true,
        );
        if kit::labelled_button(ui, kit::Icon::Loop, "Refresh games", false, true).clicked() {
            refresh_processes = true;
        }
        if let Some(warning) = &state.steam_warning {
            ui.label(RichText::new(warning).color(DANGER).size(12.5));
        }
        if state.steam_games.skipped > 0 {
            ui.label(
                RichText::new(format!(
                    "{} library entries could not be read. Existing game files are untouched.",
                    state.steam_games.skipped
                ))
                .color(MUTED)
                .size(12.5),
            );
        }
        if state.steam_games.games.is_empty() {
            ui.label(RichText::new("No installed Steam games found yet. Refresh after opening Steam or adding a library.").color(MUTED).size(12.5));
        } else {
            let query = state.builtin_game_search.trim().to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt("installed-steam-games")
                .max_height(180.0)
                .show(ui, |ui| {
                    for game in &state.steam_games.games {
                        if !game.name.to_lowercase().contains(&query)
                            && !game.app_id.contains(&query)
                        {
                            continue;
                        }
                        let mut enabled = !settings.disabled_steam_app_ids.contains(&game.app_id);
                        if ui
                            .checkbox(
                                &mut enabled,
                                format!("{} · Steam {}", game.name, game.app_id),
                            )
                            .changed()
                        {
                            if enabled {
                                settings
                                    .disabled_steam_app_ids
                                    .retain(|id| id != &game.app_id);
                            } else if settings.disabled_steam_app_ids.len()
                                < clipcraft_engine::MAX_STEAM_GAMES
                            {
                                settings.disabled_steam_app_ids.push(game.app_id.clone());
                            }
                        }
                    }
                });
        }
    });

    game_data.show(ui, &mut settings.game_integrations);
    custom_games_heading(ui);
    refresh_processes |= custom_games(ui, settings, state);
    refresh_processes
}

/// Flat settings content with room between fields; inputs keep their own borders.
pub(crate) fn game_form_section(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        ui.set_width(ui.available_width().max(0.0));
        ui.spacing_mut().item_spacing = Vec2::new(16.0, 10.0);
        ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        ui.visuals_mut().extreme_bg_color = Color32::from_rgb(13, 13, 15);
        let rounding = egui::CornerRadius::same(10);
        ui.visuals_mut().widgets.inactive.corner_radius = rounding;
        ui.visuals_mut().widgets.hovered.corner_radius = rounding;
        ui.visuals_mut().widgets.active.corner_radius = rounding;
        ui.visuals_mut().widgets.open.corner_radius = rounding;
        body(ui);
    });
}

pub(crate) fn game_field(
    ui: &mut egui::Ui,
    label: &str,
    hint: &str,
    value: &mut String,
    limit: usize,
) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 7.0;
        ui.label(
            RichText::new(label)
                .color(TEXT)
                .size(13.0)
                .family(kit::medium()),
        );
        let response = ui.add(
            egui::TextEdit::singleline(value)
                .hint_text(hint)
                .char_limit(limit)
                .margin(egui::vec2(10.0, 10.0))
                .desired_width(f32::INFINITY),
        );
        kit::name_text_box(&response, label, value);
    });
}

pub(crate) fn game_fields(
    ui: &mut egui::Ui,
    name: &mut String,
    processes: &mut String,
    steam_ids: &mut String,
) {
    if ui.available_width() >= 520.0 {
        ui.columns(2, |columns| {
            game_field(&mut columns[0], "Game name", "e.g. My game", name, 100);
            game_field(
                &mut columns[1],
                "Process names",
                "e.g. mygame.exe",
                processes,
                1600,
            );
        });
    } else {
        game_field(ui, "Game name", "e.g. My game", name, 100);
        game_field(ui, "Process names", "e.g. mygame.exe", processes, 1600);
    }
    ui.label(
        RichText::new("Use executable names, not file paths. Separate multiple names with commas.")
            .color(MUTED)
            .size(12.0),
    );
    ui.add_space(4.0);
    game_field(ui, "Steam app IDs (optional)", "e.g. 730", steam_ids, 168);
    ui.label(
        RichText::new(
            "Linux only. An exact Steam ID can distinguish games that share a process name.",
        )
        .color(MUTED)
        .size(12.0),
    );
}

pub(crate) fn custom_games(
    ui: &mut egui::Ui,
    settings: &mut RecordingSettings,
    state: &mut SettingsState,
) -> bool {
    let mut refresh_processes = false;
    card(ui, "Add a custom game", |ui| {
        game_form_section(ui, |ui| {
            ui.add_space(14.0);
            ui.label(
                RichText::new(
                    "Add a game that is missing from the built-in list, or give it your own name. Custom rules take priority.",
                )
                .color(MUTED)
                .size(12.5),
            );
            ui.add_space(12.0);
            let mut unique_processes: Vec<clipcraft_platform::RunningProcess> = Vec::new();
            for process in &state.game_processes {
                if unique_processes.iter().any(|existing| {
                    existing
                        .executable
                        .eq_ignore_ascii_case(&process.executable)
                        && existing.steam_app_id == process.steam_app_id
                }) {
                    continue;
                }
                unique_processes.push(process.clone());
                if unique_processes.len() >= 512 {
                    break;
                }
            }
            let selected_label = state
                .selected_game_process
                .as_ref()
                .map(|process| process.executable.clone())
                .unwrap_or_else(|| "Choose an app (optional)".to_owned());
            let options: Vec<(String, bool)> = unique_processes
                .iter()
                .map(|process| {
                    let label = match process.steam_app_id.as_deref() {
                        Some(id) => format!("{}  \u{b7}  Steam {id}", process.executable),
                        None => process.executable.clone(),
                    };
                    (label, true)
                })
                .collect();
            ui.horizontal(|ui| {
                if let Some(index) =
                    dropdown(ui, "Fill from a running app", &selected_label, &options)
                    && let Some(process) = unique_processes.get(index)
                {
                    state.select_game_process(process);
                }
                ui.vertical(|ui| {
                    ui.add_space(21.0);
                    if kit::labelled_button(ui, kit::Icon::Loop, "Refresh apps", false, true)
                        .clicked()
                    {
                        refresh_processes = true;
                    }
                });
            });
            if let Some(error) = state.game_process_error.as_ref() {
                ui.label(RichText::new(error).color(DANGER).size(12.5));
            } else if state.game_processes_loaded && state.game_processes.is_empty() {
                ui.label(
                    RichText::new("No running apps found. You can enter the details below.")
                        .color(MUTED)
                        .size(12.5),
                );
            } else if unique_processes.is_empty() {
                ui.label(
                    RichText::new("Refresh apps to load the list, or enter a process name below.")
                        .color(MUTED)
                        .size(12.5),
                );
            }
            ui.add_space(12.0);
            divider(ui);
            ui.add_space(14.0);
            ui.push_id("new-game", |ui| {
                game_fields(
                    ui,
                    &mut state.game_name_draft,
                    &mut state.game_process_draft,
                    &mut state.game_steam_id_draft,
                );
            });
            ui.add_space(14.0);
        });
        divider(ui);
        toggle_row(
            ui,
            "Match whole process names",
            "When off, any process name containing the entered text can match.",
            &mut state.game_exact_draft,
            true,
        );
    });
    let mut rule = GameMatchRule::new(
        state.game_name_draft.trim(),
        parse_list(&state.game_process_draft),
    )
    .with_steam_app_ids(parse_list(&state.game_steam_id_draft))
    .sanitized();
    rule.exact_executable = state.game_exact_draft;
    let invalid_steam_ids = parse_list(&state.game_steam_id_draft)
        .iter()
        .any(|id| id == "0" || id.len() > 20 || !id.bytes().all(|byte| byte.is_ascii_digit()));
    let can_add = !invalid_steam_ids
        && settings.game_rules.len() < 64
        && !rule.name.is_empty()
        && (!rule.needles.is_empty() || !rule.steam_app_ids.is_empty());
    ui.horizontal(|ui| {
        if kit::labelled_button(ui, kit::Icon::Plus, "Add custom game", true, can_add).clicked() {
            settings.game_rules.push(rule);
            state.game_name_draft.clear();
            state.game_process_draft.clear();
            state.game_steam_id_draft.clear();
            state.game_exact_draft = false;
            state.selected_game_process = None;
        }
        let (message, color) = if settings.game_rules.len() >= 64 {
            (
                "The 64-game custom list is full. Remove a rule before adding another.",
                MUTED,
            )
        } else if invalid_steam_ids {
            (
                "Steam app IDs must be nonzero numbers of at most 20 digits.",
                DANGER,
            )
        } else if !can_add {
            (
                "Enter a game name and at least one process name or Steam ID.",
                MUTED,
            )
        } else {
            ("", MUTED)
        };
        if !message.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(message).color(color).size(12.5));
        }
    });
    ui.add_space(28.0);
    card(ui, "Custom games and overrides", |ui| {
        if settings.game_rules.is_empty() {
            ui.add_space(14.0);
            ui.label(
                RichText::new(
                    "No custom rules yet. Known games are handled by the built-in list above.",
                )
                .color(MUTED)
                .size(12.5),
            );
            ui.add_space(14.0);
        }
        let mut remove = None;
        for (index, rule) in settings.game_rules.iter_mut().enumerate() {
            if index > 0 {
                divider(ui);
            }
            ui.push_id(("custom-game", index), |ui| {
                game_form_section(ui, |ui| {
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Custom rule {}", index + 1))
                                .color(TEXT)
                                .size(14.5)
                                .family(kit::semibold()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if kit::labelled_button(
                                ui,
                                kit::Icon::Trash,
                                "Remove rule",
                                false,
                                true,
                            )
                            .clicked()
                            {
                                remove = Some(index);
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let mut needles = rule.needles.join(", ");
                    let mut app_ids = rule.steam_app_ids.join(", ");
                    let before = (needles.clone(), app_ids.clone());
                    game_fields(ui, &mut rule.name, &mut needles, &mut app_ids);
                    if needles != before.0 {
                        rule.needles = parse_list(&needles);
                    }
                    if app_ids != before.1 {
                        rule.steam_app_ids = parse_list(&app_ids);
                    }
                    ui.add_space(8.0);
                });
                toggle_row(
                    ui,
                    "Match whole process names",
                    "When off, any process name containing the entered text can match.",
                    &mut rule.exact_executable,
                    true,
                );
            });
        }
        if let Some(index) = remove {
            settings.game_rules.remove(index);
        }
    });
    refresh_processes
}

pub(crate) fn set_game_presets(
    settings: &mut RecordingSettings,
    ids: Vec<String>,
    enabled: bool,
) -> Result<(), clipcraft_engine::EngineError> {
    let result = execute(Command::SetBuiltInGames {
        settings: Box::new(settings.clone()),
        ids,
        enabled,
    })?;
    match result {
        CommandOutput::RecordingSettings(updated) => {
            *settings = *updated;
            Ok(())
        }
        _ => Err(clipcraft_engine::EngineError::SettingsInvalid),
    }
}

pub(crate) fn parse_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .take(8)
        .map(str::to_owned)
        .collect()
}

pub(crate) fn display_game_name(executable: &str) -> String {
    let stem = PathBuf::from(executable)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| executable.to_owned());
    stem.replace(['_', '-'], " ")
}

/// The advanced controls stay within their normal category, below its basic settings.
pub(crate) fn custom_games_heading(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    divider(ui);
    ui.add_space(26.0);
    ui.label(
        RichText::new("Custom games")
            .color(TEXT)
            .size(20.0)
            .family(kit::semibold()),
    );
    ui.add_space(2.0);
    ui.label(
        RichText::new("Add a game ClipCraft does not know yet, by its program name.")
            .color(MUTED)
            .size(13.5),
    );
    ui.add_space(20.0);
}
