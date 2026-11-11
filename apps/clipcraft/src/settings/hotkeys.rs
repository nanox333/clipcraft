//! The Hotkeys tab: shortcut capture, extra shortcuts and clip-length shortcuts.

use super::*;

pub(crate) fn key_for_egui(key: egui::Key) -> Option<&'static str> {
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

pub(crate) fn pressed_shortcut(ui: &egui::Ui) -> Option<String> {
    ui.input(pressed_shortcut_in)
}

#[cfg(target_os = "linux")]
pub(crate) fn wayland_session(display: Option<&std::ffi::OsStr>, session: Option<&str>) -> bool {
    display.is_some_and(|value| !value.is_empty())
        || session.is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
}

pub fn hotkeys_tab(
    ui: &mut egui::Ui,
    settings: &mut RecordingSettings,
    state: &mut SettingsState,
    status: Option<&clipcraft_platform::HotkeyStatus>,
) -> bool {
    #[cfg(target_os = "linux")]
    let mut retry = false;
    #[cfg(not(target_os = "linux"))]
    let retry = false;
    if state.capturing.is_some() {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            state.capturing = None;
        } else if let (Some((action, index)), Some(shortcut)) =
            (state.capturing, pressed_shortcut(ui))
        {
            state.hotkey_error = settings.hotkeys.set_binding(action, index, &shortcut).err();
            state.capturing = None;
        }
    }
    if let Some(index) = state.capturing_clip_length {
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            state.capturing_clip_length = None;
        } else if let Some(shortcut) = pressed_shortcut(ui) {
            state.hotkey_error = settings.hotkeys.set_clip_length_key(index, &shortcut).err();
            state.capturing_clip_length = None;
        }
    }
    card(ui, "Shortcuts", |ui| {
        #[cfg(target_os = "linux")]
        if wayland_session(
            std::env::var_os("WAYLAND_DISPLAY").as_deref(),
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        ) {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Wayland asks your desktop to register shortcuts. The desktop may choose different keys; its current bindings are shown below.")
                    .color(MUTED)
                    .size(12.5),
            );
            let status_text = match status {
                Some(clipcraft_platform::HotkeyStatus::Pending) => {
                    "Waiting for the desktop shortcut dialog…"
                }
                Some(clipcraft_platform::HotkeyStatus::Portal { .. }) => {
                    "Desktop shortcuts are connected."
                }
                Some(clipcraft_platform::HotkeyStatus::Fallback {
                    direct_keyboard: true,
                    ..
                }) => "Using the keyboard-device fallback.",
                Some(clipcraft_platform::HotkeyStatus::Fallback {
                    direct_keyboard: false,
                    ..
                }) => "Using limited XWayland shortcuts.",
                Some(clipcraft_platform::HotkeyStatus::Unavailable { .. }) => {
                    "Global shortcuts are unavailable. Some keys work while ClipCraft is focused; push-to-talk needs global access."
                }
                _ => "Global shortcut status is not available yet.",
            };
            ui.label(RichText::new(status_text).color(TEXT).size(12.5));
            if matches!(
                status,
                Some(
                    clipcraft_platform::HotkeyStatus::Fallback { .. }
                        | clipcraft_platform::HotkeyStatus::Unavailable { .. }
                )
            ) && kit::labelled_button(
                ui,
                kit::Icon::Loop,
                "Retry desktop shortcuts",
                false,
                true,
            )
            .clicked()
            {
                retry = true;
            }
            if let Some(
                clipcraft_platform::HotkeyStatus::Portal { unbound, .. }
                | clipcraft_platform::HotkeyStatus::Fallback { unbound, .. },
            ) = status
                && !unbound.is_empty()
            {
                ui.label(
                    RichText::new(format!("Not registered globally: {}", unbound.join(", ")))
                        .color(DANGER)
                        .size(12.5),
                );
            }
            ui.add_space(6.0);
            ui.checkbox(
                &mut settings.wayland_raw_hotkeys,
                "Allow direct keyboard access if desktop shortcuts fail",
            );
            ui.label(RichText::new("Off by default. This fallback requires keyboard-device permission; other programs with the same access can read your keystrokes.").color(MUTED).size(12.0));
        }
        let count = HotkeyAction::ALL.len();
        for (index, action) in HotkeyAction::ALL.into_iter().enumerate() {
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(action.label())
                            .color(TEXT)
                            .size(14.5)
                            .family(kit::semibold()),
                    );
                    ui.label(RichText::new(action.detail()).color(MUTED).size(12.5));
                    if let Some(clipcraft_platform::HotkeyStatus::Portal { assigned, .. }) = status
                    {
                        for (preferred, actual) in assigned.iter().filter(|(key, _)| {
                            settings
                                .hotkeys
                                .bindings(action)
                                .any(|binding| binding == key)
                        }) {
                            ui.label(
                                RichText::new(format!("Desktop: {actual} (requested {preferred})"))
                                    .color(MUTED)
                                    .size(12.0),
                            );
                        }
                    }
                    if let Some(other) = settings.hotkeys.conflict(action) {
                        ui.label(
                            RichText::new(format!("Also used by “{}”.", other.label()))
                                .color(DANGER)
                                .size(12.5),
                        );
                    }
                });
                ui.add_space(6.0);
                let bindings: Vec<String> = settings
                    .hotkeys
                    .bindings(action)
                    .map(str::to_owned)
                    .collect();
                let count = bindings.len();
                let mut remove = None;
                ui.horizontal_wrapped(|ui| {
                    for (binding_index, current) in bindings.iter().enumerate() {
                        let waiting = state.capturing == Some((action, binding_index));
                        let text = if waiting {
                            "Press a key…".to_owned()
                        } else if current.is_empty() {
                            "Not set".to_owned()
                        } else {
                            current.replace('+', " + ")
                        };
                        let response = ui.add(
                            egui::Button::new(RichText::new(&text).color(if waiting {
                                LIME
                            } else {
                                TEXT
                            }))
                            .fill(Color32::from_rgb(13, 13, 15))
                            .stroke(Stroke::new(1.0_f32, if waiting { LIME } else { BORDER }))
                            .min_size(Vec2::new(150.0, 34.0)),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!(
                                    "{} shortcut {}: {}",
                                    action.label(),
                                    binding_index + 1,
                                    text
                                ),
                            )
                        });
                        if response
                            .on_hover_text("Click, then press the keys")
                            .clicked()
                        {
                            state.capturing = if waiting {
                                None
                            } else {
                                Some((action, binding_index))
                            };
                            state.hotkey_error = None;
                        }
                        let clear = ui.add_enabled(
                            !current.is_empty(),
                            egui::Button::new(RichText::new("×").color(MUTED)).frame(false),
                        );
                        clear.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("Remove {} shortcut {}", action.label(), binding_index + 1),
                            )
                        });
                        if clear
                            .on_hover_text(format!(
                                "Remove {} shortcut {}",
                                action.label(),
                                binding_index + 1
                            ))
                            .clicked()
                        {
                            remove = Some(binding_index);
                        }
                    }
                    let adding = state.capturing == Some((action, count));
                    let add = ui.add_enabled(
                        count < clipcraft_engine::Hotkeys::MAX_BINDINGS,
                        egui::Button::new(if adding { "Press a key…" } else { "+" })
                            .min_size(Vec2::new(34.0, 34.0)),
                    );
                    add.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            count < 5,
                            format!("Add shortcut for {}", action.label()),
                        )
                    });
                    if add
                        .on_hover_text(format!("Add shortcut for {} ({count}/5)", action.label()))
                        .clicked()
                    {
                        state.capturing = if adding { None } else { Some((action, count)) };
                        state.hotkey_error = None;
                    }
                    ui.label(RichText::new(format!("{count}/5")).color(FAINT).size(12.0));
                });
                if let Some(index) = remove {
                    settings.hotkeys.remove_binding(action, index);
                    state.capturing = None;
                }
            });
            ui.add_space(14.0);
            if index + 1 < count {
                divider(ui);
            }
        }
    });
    clip_length_card(ui, settings, state);
    if let Some(error) = state.hotkey_error.as_ref() {
        ui.colored_label(DANGER, error);
    }
    ui.label(
        RichText::new("Click a shortcut, then press the keys you want. Press Esc to cancel.")
            .color(FAINT)
            .size(12.5),
    );
    ui.add_space(12.0);
    if kit::labelled_button(ui, kit::Icon::Loop, "Reset to defaults", false, true).clicked() {
        settings.hotkeys = clipcraft_engine::Hotkeys::default();
        state.capturing = None;
        state.capturing_clip_length = None;
        state.hotkey_error = None;
    }
    ui.add_space(8.0);
    retry
}

/// Shortcuts that each save their own length of the replay, such as the last 2 minutes.
pub(crate) fn clip_length_card(
    ui: &mut egui::Ui,
    settings: &mut RecordingSettings,
    state: &mut SettingsState,
) {
    use clipcraft_engine::{CLIP_LENGTH_CHOICES, MAX_CLIP_LENGTH_SHORTCUTS, describe_seconds};
    card(ui, "Clip-length shortcuts", |ui| {
        ui.label(
            RichText::new("Give a key its own clip length, for example Shift + F8 for the last 2 minutes. Save clip keeps the replay length.")
                .color(MUTED)
                .size(12.5),
        );
        let replay = settings.replay_seconds;
        let mut remove = None;
        let shortcuts = settings.hotkeys.clip_lengths.clone();
        for (index, shortcut) in shortcuts.iter().enumerate() {
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let waiting = state.capturing_clip_length == Some(index);
                let text = if waiting {
                    "Press a key…".to_owned()
                } else if shortcut.key.is_empty() {
                    "Choose a key".to_owned()
                } else {
                    shortcut.key.replace('+', " + ")
                };
                let response = ui.add(
                    egui::Button::new(RichText::new(&text).color(if waiting {
                        LIME
                    } else {
                        TEXT
                    }))
                    .fill(Color32::from_rgb(13, 13, 15))
                    .stroke(Stroke::new(1.0_f32, if waiting { LIME } else { BORDER }))
                    .min_size(Vec2::new(150.0, 34.0)),
                );
                let length = describe_seconds(shortcut.seconds);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("Shortcut for saving the last {length}: {text}"),
                    )
                });
                if response
                    .on_hover_text("Click, then press the keys")
                    .clicked()
                {
                    state.capturing_clip_length = if waiting { None } else { Some(index) };
                    state.capturing = None;
                    state.hotkey_error = None;
                }
                ui.label(RichText::new("saves the last").color(MUTED).size(12.5));
                for seconds in CLIP_LENGTH_CHOICES {
                    if choice(
                        ui,
                        &describe_seconds(seconds),
                        shortcut.seconds == seconds,
                        true,
                        "Clip length",
                    ) {
                        settings.hotkeys.set_clip_length_seconds(index, seconds);
                    }
                }
                let delete =
                    ui.add(egui::Button::new(RichText::new("×").color(MUTED)).frame(false));
                delete.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        format!("Remove the {length} clip shortcut"),
                    )
                });
                if delete
                    .on_hover_text(format!("Remove the {length} clip shortcut"))
                    .clicked()
                {
                    remove = Some(index);
                }
            });
            if shortcut.seconds > replay {
                ui.label(
                    RichText::new(format!(
                        "The replay buffer holds {}, so this saves at most that. Raise the replay length in Video to keep more.",
                        describe_seconds(replay)
                    ))
                    .color(MUTED)
                    .size(12.0),
                );
            }
        }
        if let Some(index) = remove {
            settings.hotkeys.remove_clip_length(index);
            state.capturing_clip_length = None;
        }
        ui.add_space(10.0);
        let count = settings.hotkeys.clip_lengths.len();
        let add = ui.add_enabled(
            count < MAX_CLIP_LENGTH_SHORTCUTS,
            egui::Button::new("+ Add a clip-length shortcut").min_size(Vec2::new(34.0, 34.0)),
        );
        add.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                count < MAX_CLIP_LENGTH_SHORTCUTS,
                "Add a clip-length shortcut",
            )
        });
        if add
            .on_hover_text(format!("{count}/{MAX_CLIP_LENGTH_SHORTCUTS}"))
            .clicked()
            && let Ok(index) = settings.hotkeys.add_clip_length(120)
        {
            // Ask for its keys straight away.
            state.capturing_clip_length = Some(index);
            state.capturing = None;
            state.hotkey_error = None;
        }
    });
}
