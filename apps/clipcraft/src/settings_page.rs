//! The Settings page shell: category list, search, the unsaved-changes bar and the General tab.
//! The other tabs live in `settings`.

use super::*;

impl ClipCraftApp {
    pub(crate) fn settings(&mut self, ui: &mut egui::Ui) {
        let height = ui.available_height().max(0.0);
        let nav_width = 210.0_f32.min(ui.available_width() * 0.35).max(0.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.allocate_ui_with_layout(
                Vec2::new(nav_width, height),
                Layout::top_down(Align::Min),
                |ui| self.settings_nav(ui),
            );
            let content_width = ui.available_width().max(0.0);
            ui.allocate_ui_with_layout(
                Vec2::new(content_width, height),
                Layout::top_down(Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("settings-content", self.settings_tab.label()))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let column = (content_width - 56.0).clamp(0.0, 720.0);
                            ui.horizontal_top(|ui| {
                                ui.add_space(((content_width - column) / 2.0).max(0.0));
                                ui.vertical(|ui| {
                                    ui.set_width(column);
                                    self.settings_content(ui);
                                });
                            });
                        });
                },
            );
        });
    }

    /// The category list on the left of Settings, with a search that finds any setting.
    pub(crate) fn settings_nav(&mut self, ui: &mut egui::Ui) {
        ui.set_width(ui.available_width());
        ui.add_space(12.0);
        ui.allocate_ui(
            Vec2::new((ui.available_width() - 12.0).max(0.0), 40.0),
            |ui| {
                let response =
                    settings::search_box(ui, &mut self.settings_search, "Search settings");
                kit::name_text_box(&response, "Search settings", &self.settings_search);
            },
        );
        ui.add_space(8.0);
        let searching = !self.settings_search.trim().is_empty();
        let groups: [(&str, Vec<SettingsTab>); 3] = [
            ("", vec![SettingsTab::Overview]),
            (
                "Recording",
                vec![
                    SettingsTab::Capture,
                    SettingsTab::Video,
                    SettingsTab::Audio,
                    SettingsTab::Replay,
                    SettingsTab::Games,
                ],
            ),
            (
                "Settings",
                vec![
                    SettingsTab::General,
                    SettingsTab::Storage,
                    SettingsTab::Hotkeys,
                    SettingsTab::About,
                ],
            ),
        ];
        for (group, tabs) in groups {
            if !group.is_empty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(group.to_uppercase())
                        .color(FAINT)
                        .size(11.5)
                        .family(kit::semibold()),
                );
                ui.add_space(4.0);
            }
            for tab in tabs {
                let selected = !searching && self.settings_tab == tab;
                if self.settings_nav_row(ui, tab.label(), selected) {
                    self.settings_tab = tab;
                    self.settings_search.clear();
                }
            }
        }
    }

    /// One row in the settings list.
    pub(crate) fn settings_nav_row(&self, ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new((ui.available_width() - 12.0).max(0.0), 34.0),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.to_owned())
        });
        if selected {
            ui.painter().rect_filled(rect, 8.0, kit::CARD_HOVER);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 8.0, kit::CARD);
        }
        ui.painter().text(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::new(14.0, kit::medium()),
            if selected { TEXT } else { MUTED },
        );
        response.clicked()
    }

    /// The search results: every matching row, drawn in place so it can be changed right there,
    /// then links to the pages whose settings are not rows.
    pub(crate) fn settings_search_results(&mut self, ui: &mut egui::Ui, query: &str) {
        let before = self.recording.clone();
        let mut working = before.clone();
        let mut refresh = false;
        let shown = {
            let gpu = self.capture.gpu.clone();
            let mut env = settings::Env {
                gpu: &gpu,
                capabilities: clipcraft_platform::capabilities(),
                targets: &self.capture.targets,
                refresh_targets: &mut refresh,
            };
            settings::search_results(ui, query, &mut working, &mut env, &mut self.settings_state)
        };
        if refresh {
            self.capture.refresh_targets();
        }
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let others: Vec<(&str, SettingsTab)> = SETTINGS_INDEX
            .iter()
            .filter(|(title, keywords, tab)| {
                let haystack = format!("{title} {keywords} {}", tab.label()).to_lowercase();
                words.iter().all(|word| haystack.contains(word))
            })
            .map(|(title, _, tab)| (*title, *tab))
            .collect();
        if shown == 0 && others.is_empty() {
            ui.label(
                RichText::new("No setting matches that.")
                    .color(MUTED)
                    .size(14.0),
            );
        }
        if !others.is_empty() {
            ui.label(
                RichText::new("Also on these pages")
                    .color(TEXT)
                    .size(15.5)
                    .family(kit::semibold()),
            );
            ui.add_space(8.0);
            for (title, tab) in others {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 46.0),
                    egui::Sense::click(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title.to_owned())
                });
                ui.painter().rect_filled(
                    rect,
                    12.0,
                    if response.hovered() {
                        kit::CARD_HOVER
                    } else {
                        kit::CARD
                    },
                );
                ui.painter().text(
                    egui::pos2(rect.left() + 18.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    title,
                    egui::FontId::new(14.0, kit::medium()),
                    TEXT,
                );
                ui.painter().text(
                    egui::pos2(rect.right() - 18.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    format!("{} \u{203a}", tab.label()),
                    egui::FontId::proportional(12.5),
                    MUTED,
                );
                if response.clicked() {
                    self.settings_tab = tab;
                    self.settings_search.clear();
                }
                ui.add_space(6.0);
            }
        }
        self.apply_recording(before, working);
    }

    /// Settings apply as you change them; this bar lets you take back everything changed since
    /// the Settings page was opened.
    pub(crate) fn unsaved_changes_bar(&mut self, ui: &mut egui::Ui) {
        let snapshot = self
            .settings_snapshot
            .get_or_insert_with(|| self.recording.clone())
            .clone();
        if snapshot == self.recording {
            return;
        }
        let (mut cancel, mut keep) = (false, false);
        egui::Frame::new()
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("You changed some settings")
                            .color(TEXT)
                            .size(14.0)
                            .family(kit::semibold()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        cancel = kit::labelled_button(
                            ui,
                            Icon::Restart,
                            "Cancel changes",
                            false,
                            true,
                        )
                        .on_hover_text(
                            "Put every setting back the way it was when you opened Settings",
                        )
                        .clicked();
                        keep = kit::labelled_button(ui, Icon::Play, "Keep changes", true, true)
                            .clicked();
                    });
                });
            });
        ui.add_space(12.0);
        if cancel {
            let before = self.recording.clone();
            self.apply_recording(before, snapshot);
            self.settings_snapshot = None;
        } else if keep {
            self.settings_snapshot = Some(self.recording.clone());
        }
    }

    pub(crate) fn settings_content(&mut self, ui: &mut egui::Ui) {
        let query = self.settings_search.trim().to_owned();
        ui.add_space(22.0);
        ui.label(
            RichText::new(if query.is_empty() {
                self.settings_tab.label()
            } else {
                "Search"
            })
            .color(TEXT)
            .size(28.0)
            .family(kit::semibold()),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(if query.is_empty() {
                self.settings_tab.help().to_owned()
            } else {
                format!("Results for \u{201c}{query}\u{201d}")
            })
            .color(MUTED)
            .size(14.0),
        );
        ui.add_space(24.0);
        self.unsaved_changes_bar(ui);
        if self.capture.is_stale(&self.recording) {
            let mut restart = false;
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(190, 255, 43, 14))
                .stroke(Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(190, 255, 43, 70),
                ))
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::symmetric(16, 14))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        // Room for the button on the right; the words take the rest.
                        let words = (ui.available_width() - 190.0).max(120.0);
                        ui.vertical(|ui| {
                            ui.set_width(words);
                            ui.label(
                                RichText::new("New settings are ready")
                                    .color(TEXT)
                                    .size(14.5)
                                    .family(kit::semibold()),
                            );
                            ui.label(
                                RichText::new(
                                    "Clipping is still using the earlier settings. Restarting applies them and empties the replay buffer.",
                                )
                                .color(MUTED)
                                .size(12.5),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            restart =
                                kit::labelled_button(ui, Icon::Restart, "Restart clipping", true, true)
                                    .on_hover_text("Stop clipping and start again with the new settings")
                                    .clicked();
                        });
                    });
                });
            ui.add_space(12.0);
            if restart {
                self.stop_capture();
                self.start_capture();
            }
        }
        if !query.is_empty() {
            self.settings_search_results(ui, &query);
            ui.add_space(8.0);
            return;
        }
        let tab = self.settings_tab;
        self.settings_state
            .leave(tab == SettingsTab::Audio, tab == SettingsTab::Hotkeys);
        let before = self.recording.clone();
        let mut working = before.clone();
        match tab {
            SettingsTab::Overview => {
                let live = self
                    .capture
                    .record_status()
                    .map(|status| settings::Live {
                        recording: self.capture.is_active(),
                        captured: status.frames_captured,
                        dropped: status.frames_dropped,
                        encode_ms: status.average_encode_ms,
                        target_fps: status.target_fps,
                    })
                    .unwrap_or(settings::Live {
                        recording: false,
                        captured: 0,
                        dropped: 0,
                        encode_ms: 0.0,
                        target_fps: self.recording.frames_per_second,
                    });
                settings::overview(ui, &mut working, &self.capture.gpu, &live);
            }
            SettingsTab::General => self.general_settings(ui, &mut working),
            SettingsTab::Storage => {
                let folder = self.clips_folder();
                let mut browse = false;
                settings::storage_tab(
                    ui,
                    &mut working,
                    &mut self.settings_state,
                    folder,
                    &mut browse,
                );
                if browse {
                    self.choose_library_folder();
                }
            }
            SettingsTab::Hotkeys => {
                let status = self.capture.shortcut_status();
                if settings::hotkeys_tab(
                    ui,
                    &mut working,
                    &mut self.settings_state,
                    status.as_ref(),
                ) {
                    self.capture.retry_shortcuts();
                    ui.ctx().request_repaint();
                }
            }
            SettingsTab::Games => {
                let refresh = settings::games_tab(
                    ui,
                    &mut working,
                    &mut self.settings_state,
                    &mut self.game_data,
                );
                if refresh {
                    self.request_game_refresh();
                }
            }
            SettingsTab::Capture => {
                let mut refresh = !self.capture.targets_loaded();
                settings::capture_tab(
                    ui,
                    &mut working,
                    &self.capture.targets,
                    &mut refresh,
                    clipcraft_platform::capabilities(),
                );
                if refresh {
                    self.capture.refresh_targets();
                }
            }
            SettingsTab::Video => settings::video_tab(ui, &mut working, &self.capture.gpu),
            SettingsTab::Audio => settings::audio_tab(ui, &mut working, &mut self.settings_state),
            SettingsTab::Replay => settings::replay_tab(ui, &mut working),
            SettingsTab::About => {
                settings::about_tab(ui);
                self.bug_report_card(ui);
                self.updates_card(ui);
            }
        }
        self.apply_recording(before, working);
        ui.add_space(8.0);
    }

    pub(crate) fn general_settings(&mut self, ui: &mut egui::Ui, working: &mut RecordingSettings) {
        let folder = if self.directory_text.trim().is_empty() {
            match self.clips_folder() {
                Some(default) => format!("{} (default)", default.to_string_lossy()),
                None => "No folder chosen yet".to_owned(),
            }
        } else {
            self.directory_text.clone()
        };
        let mut browse = false;
        card(ui, "Library", |ui| {
            setting_row(
                ui,
                "Clips folder",
                "Your clips live here: ClipCraft shows the videos in this folder, and new clips are saved into it. Nothing is moved or changed.",
                true,
                |ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(13, 13, 15))
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            // An exact row height: a centered layout would otherwise claim all
                            // the height a scroll area offers.
                            ui.allocate_ui_with_layout(
                                Vec2::new(ui.available_width(), 34.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    let (icon, _) = ui.allocate_exact_size(
                                        Vec2::splat(20.0),
                                        egui::Sense::hover(),
                                    );
                                    kit::draw_icon(
                                        ui.painter(),
                                        icon.center(),
                                        Icon::Folder,
                                        MUTED,
                                        0.8,
                                    );
                                    ui.add(
                                        egui::Label::new(RichText::new(folder).color(TEXT))
                                            .truncate(),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        browse = kit::labelled_button(
                                            ui,
                                            Icon::Folder,
                                            "Browse",
                                            false,
                                            true,
                                        )
                                        .clicked();
                                    });
                                },
                            );
                        });
                },
            );
        });
        if browse {
            self.choose_library_folder();
        }
        settings::startup_card(ui, working);
        self.presence.show(ui, &mut working.discord_presence);
        card(ui, "Help", |ui| {
            setting_row(
                ui,
                "Copy a diagnostics report",
                "If recording does not work, this copies what ClipCraft can see: your folders, monitors, sound devices, and video encoders. Nothing is sent anywhere; paste it where you ask for help.",
                true,
                |ui| {
                    let label = if self
                        .diagnostics_copied
                        .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(4))
                    {
                        "Copied"
                    } else {
                        "Copy report"
                    };
                    if kit::labelled_button(ui, Icon::Copy, label, false, true).clicked() {
                        let text = self.diagnostics_text();
                        ui.ctx().copy_text(text);
                        self.diagnostics_copied = Some(std::time::Instant::now());
                    }
                },
            );
        });
    }

    /// The diagnostics report as text, from the saved settings and what this computer shows now.
    pub(crate) fn diagnostics_text(&self) -> String {
        let settings = &self.recording;
        let mut report = clipcraft_engine::Diagnostics::collect(settings);
        let check = |(name, ok, detail): clipcraft_platform::Finding| clipcraft_engine::Check {
            name,
            ok,
            detail,
        };
        report.capture = clipcraft_platform::diagnose_capture()
            .into_iter()
            .map(check)
            .collect();
        report.video = clipcraft_platform::diagnose_video()
            .into_iter()
            .map(check)
            .collect();
        let devices = clipcraft_audio_out::list_devices();
        for (kind, names) in [
            ("Microphones", &devices.inputs),
            ("Speakers", &devices.outputs),
        ] {
            report.audio.push(if names.is_empty() {
                clipcraft_engine::Check::fail(kind, "none found")
            } else {
                clipcraft_engine::Check::pass(kind, names.join(", "))
            });
        }
        report
            .audio
            .push(check(clipcraft_platform::diagnose_system_sound()));
        report.text()
    }

    /// Keeps the settings changed on screen: applies start-up changes, then saves.
    pub(crate) fn apply_recording(
        &mut self,
        before: RecordingSettings,
        working: RecordingSettings,
    ) {
        let mut working = working.sanitized();
        if working == before {
            return;
        }
        if working.launch_at_startup != before.launch_at_startup {
            let executable = std::env::current_exe().unwrap_or_default();
            if let Err(error) = execute(Command::SetLaunchAtStartup {
                enabled: working.launch_at_startup,
                executable,
            }) {
                self.error = Some(error.to_string());
                working.launch_at_startup = before.launch_at_startup;
            }
        }
        self.recording = working;
        self.save_recording_settings();
    }

    pub(crate) fn save_recording_settings(&mut self) {
        let Some(path) = self.settings_path.clone() else {
            return;
        };
        match execute(Command::SaveRecordingSettings {
            path,
            settings: Box::new(self.recording.clone()),
        }) {
            Ok(CommandOutput::RecordingSettings(stored)) => self.recording = *stored,
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}
