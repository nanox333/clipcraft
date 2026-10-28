//! Looking for a newer ClipCraft, offering it, and handing it to the setup program (ADR 0032).
//!
//! The check is one small request for the latest published release and is off when the person
//! turns "Check for updates" off. Nothing is downloaded until they press Update.

use std::sync::mpsc;
use std::time::Duration;

use clipcraft_engine::update::{UpdateInfo, UpdateStatus};
use clipcraft_engine::{Command, CommandOutput, EngineError, execute};
use eframe::egui::{self, Align, Layout, RichText, Stroke};

use crate::kit::{self, BORDER, CARD, MUTED, TEXT};
use crate::{ClipCraftApp, Icon};

/// This build's version, compared with the latest published one.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// What the update check and download are doing.
#[derive(Default)]
pub(crate) enum Updater {
    /// Nothing has been asked yet.
    #[default]
    Idle,
    /// Asking whether a newer version exists.
    Checking(mpsc::Receiver<Result<UpdateStatus, EngineError>>),
    /// This is the newest version.
    UpToDate,
    /// A newer version exists.
    Available(UpdateInfo),
    /// Fetching and verifying the setup program.
    Downloading(UpdateInfo, mpsc::Receiver<Result<String, EngineError>>),
    /// The check or download failed.
    Failed(String),
}

impl Updater {
    /// A line for the About page.
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Idle => format!("You have version {CURRENT}."),
            Self::Checking(_) => "Checking for updates\u{2026}".to_owned(),
            Self::UpToDate => format!("You have the newest version ({CURRENT})."),
            Self::Available(update) => format!("Version {} is available.", update.version),
            Self::Downloading(update, _) => {
                format!("Downloading version {}\u{2026}", update.version)
            }
            Self::Failed(why) => why.clone(),
        }
    }

    fn busy(&self) -> bool {
        matches!(self, Self::Checking(_) | Self::Downloading(..))
    }
}

impl ClipCraftApp {
    /// Starts asking whether a newer version exists. Ignored while that is already happening.
    pub(crate) fn check_for_update(&mut self, context: &egui::Context) {
        if self.updater.busy() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let wake = context.clone();
        std::thread::spawn(move || {
            let _ = sender.send(
                execute(Command::CheckForUpdate {
                    current_version: CURRENT.to_owned(),
                })
                .and_then(|output| match output {
                    CommandOutput::Update(status) => Ok(status),
                    _ => Err(EngineError::UpdateInvalid),
                }),
            );
            wake.request_repaint();
        });
        self.updater = Updater::Checking(receiver);
    }

    /// Downloads the update, then (see `poll_update`) hands it to the setup program.
    fn download_update(&mut self, update: UpdateInfo, context: &egui::Context) {
        let (sender, receiver) = mpsc::channel();
        let wake = context.clone();
        let wanted = update.clone();
        std::thread::spawn(move || {
            let _ = sender.send(
                execute(Command::DownloadUpdate { update: wanted }).and_then(
                    |output| match output {
                        CommandOutput::UpdateDownloaded { path } => Ok(path),
                        _ => Err(EngineError::UpdateInvalid),
                    },
                ),
            );
            wake.request_repaint();
        });
        self.updater = Updater::Downloading(update, receiver);
    }

    /// Once per frame: the automatic check on start-up, and answers from the worker threads.
    pub(crate) fn poll_update(&mut self, context: &egui::Context) {
        if !self.update_checked_on_launch {
            self.update_checked_on_launch = true;
            if self.recording.check_for_updates && self.recording.setup_completed {
                self.check_for_update(context);
            }
        }
        let next = match &self.updater {
            Updater::Checking(receiver) => match receiver.try_recv() {
                Ok(Ok(UpdateStatus::UpToDate)) => Some(Updater::UpToDate),
                Ok(Ok(UpdateStatus::Available(update))) => Some(Updater::Available(update)),
                Ok(Err(error)) => Some(Updater::Failed(error.to_string())),
                Err(mpsc::TryRecvError::Empty) => {
                    context.request_repaint_after(Duration::from_millis(300));
                    None
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Updater::Failed(EngineError::UpdateUnavailable.to_string()))
                }
            },
            Updater::Downloading(update, receiver) => match receiver.try_recv() {
                Ok(Ok(path)) => {
                    // The setup program replaces this version; it needs ClipCraft closed.
                    match std::process::Command::new(&path).spawn() {
                        Ok(_) => {
                            crate::tray::Tray::quit(context);
                            Some(Updater::Available(update.clone()))
                        }
                        Err(_) => Some(Updater::Failed(format!(
                            "The update was downloaded to {path} but could not be started. Run it yourself."
                        ))),
                    }
                }
                Ok(Err(error)) => Some(Updater::Failed(error.to_string())),
                Err(mpsc::TryRecvError::Empty) => {
                    context.request_repaint_after(Duration::from_millis(300));
                    None
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Updater::Failed(EngineError::UpdateUnavailable.to_string()))
                }
            },
            _ => None,
        };
        if let Some(next) = next {
            self.updater = next;
        }
    }

    /// The strip across the top of the window while a newer version is waiting.
    pub(crate) fn update_banner(&mut self, ui: &mut egui::Ui) {
        let Updater::Available(update) = &self.updater else {
            return;
        };
        if self.update_dismissed.as_deref() == Some(update.version.as_str()) {
            return;
        }
        let update = update.clone();
        let (mut install, mut later, mut notes) = (false, false, false);
        egui::Frame::new()
            .fill(egui::Color32::from_rgba_unmultiplied(190, 255, 43, 18))
            .stroke(Stroke::new(
                1.0_f32,
                egui::Color32::from_rgba_unmultiplied(190, 255, 43, 80),
            ))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::symmetric(16, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("ClipCraft {} is available", update.version))
                            .color(TEXT)
                            .size(14.0)
                            .family(kit::semibold()),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        later = kit::labelled_button(ui, Icon::Restart, "Later", false, true)
                            .clicked();
                        notes = kit::labelled_button(ui, Icon::Search, "What\u{2019}s new", false, true)
                            .clicked();
                        install = kit::labelled_button(ui, Icon::Import, "Update now", true, true)
                            .on_hover_text(
                                "Downloads the update, checks it, then closes ClipCraft and opens its setup. Your clips are not touched.",
                            )
                            .clicked();
                    });
                });
            });
        ui.add_space(10.0);
        if install {
            self.download_update(update, ui.ctx());
        } else if later {
            self.update_dismissed = Some(update.version);
        } else if notes {
            ui.ctx().open_url(egui::OpenUrl::new_tab(update.page_url));
        }
    }

    /// The "Updates" card on the About page.
    pub(crate) fn updates_card(&mut self, ui: &mut egui::Ui) {
        let mut check = false;
        let mut install = None;
        egui::Frame::new()
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(egui::CornerRadius::same(14))
            .inner_margin(egui::Margin::symmetric(20, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("Updates")
                        .color(TEXT)
                        .size(15.0)
                        .family(kit::semibold()),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.updater.summary()).color(MUTED).size(13.5));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Updater::Available(update) = &self.updater {
                            if kit::labelled_button(ui, Icon::Import, "Update now", true, true)
                                .clicked()
                            {
                                install = Some(update.clone());
                            }
                        } else if kit::labelled_button(
                            ui,
                            Icon::Restart,
                            "Check for updates",
                            false,
                            !self.updater.busy(),
                        )
                        .clicked()
                        {
                            check = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Checking asks GitHub for the newest published version and sends nothing about you. You can turn the automatic check off in Settings \u{203a} General.",
                    )
                    .color(MUTED)
                    .size(12.5),
                );
            });
        ui.add_space(12.0);
        if check {
            self.check_for_update(ui.ctx());
        }
        if let Some(update) = install {
            self.download_update(update, ui.ctx());
        }
    }
}
