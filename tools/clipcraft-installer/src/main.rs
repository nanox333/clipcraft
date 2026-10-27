#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Native offline installer. GUI and explicit unattended arguments share engine commands.
use clipcraft_engine::{Command, CommandOutput, execute};
use eframe::egui;
use std::{path::PathBuf, sync::mpsc};

fn default_destination() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Programs/ClipCraft"))
}

fn install(destination: PathBuf, integrate: bool, desktop: bool) -> Result<PathBuf, String> {
    let bundle = std::env::current_exe().map_err(|_| "Cannot locate this installer.")?;
    match execute(Command::InstallApplication {
        bundle,
        destination: destination.clone(),
    }) {
        Ok(CommandOutput::Installation(exe)) => {
            if integrate {
                let uninstaller = exe
                    .parent()
                    .and_then(|p| p.parent())
                    .ok_or("Invalid application layout.")?
                    .join("uninstall.exe");
                if let Err(error) = clipcraft_platform::register_installation(
                    &destination,
                    &exe,
                    &uninstaller,
                    desktop,
                ) {
                    return Err(format!(
                        "The app was installed. Windows shortcuts / app registration failed: {error}. You can launch bin/clipcraft.exe in the newest release folder."
                    ));
                }
            }
            Ok(exe)
        }
        Ok(_) => Err("The installer returned an unexpected result.".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn uninstall(destination: PathBuf, integrate: bool) -> Result<PathBuf, String> {
    match execute(Command::UninstallApplication {
        destination: destination.clone(),
    }) {
        Ok(CommandOutput::Installation(path)) => {
            if integrate {
                clipcraft_platform::unregister_installation(&destination)?;
            }
            Ok(path)
        }
        Ok(_) => Err("The uninstaller returned an unexpected result.".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    // Explicit unattended paths are useful to managed deployments and isolated packaging tests.
    if arguments.first().is_some_and(|a| a == "--install-to") {
        if arguments.len() != 4
            || arguments[2] != "--accept-license"
            || arguments[3] != "--no-integration"
        {
            return Err("Usage: clipcraft-setup --install-to <absolute-folder> --accept-license --no-integration".into());
        }
        install(PathBuf::from(&arguments[1]), false, false)?;
        return Ok(());
    }
    if arguments.first().is_some_and(|a| a == "--remove-from") {
        if arguments.len() != 3 || arguments[2] != "--no-integration" {
            return Err("Usage: uninstall --remove-from <absolute-folder> --no-integration".into());
        }
        uninstall(PathBuf::from(&arguments[1]), false)?;
        return Ok(());
    }
    let current = std::env::current_exe().map_err(|_| "Cannot locate this executable.")?;
    let is_uninstaller = current
        .file_name()
        .is_some_and(|name| name == "uninstall.exe");
    let destination = if is_uninstaller {
        let destination = current
            .parent()
            .and_then(|p| p.parent())
            .ok_or("Invalid uninstaller location.")?
            .to_path_buf();
        // Windows cannot remove a running executable. A uniquely created temporary copy owns the UI.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "The system clock is invalid.")?
            .as_nanos();
        let temp = std::env::temp_dir().join(format!(
            "clipcraft-uninstall-{}-{stamp}.exe",
            std::process::id()
        ));
        let mut output = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        std::io::copy(
            &mut std::fs::File::open(&current).map_err(|e| e.to_string())?,
            &mut output,
        )
        .map_err(|e| e.to_string())?;
        drop(output);
        std::process::Command::new(&temp)
            .arg("--uninstall-ui")
            .arg(&destination)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    } else if arguments.first().is_some_and(|a| a == "--uninstall-ui") && arguments.len() == 2 {
        PathBuf::from(&arguments[1])
    } else if arguments.is_empty() {
        default_destination().ok_or("Windows Local App Data is unavailable.")?
    } else {
        return Err("Unsupported installer arguments.".into());
    };
    let removing = arguments.first().is_some_and(|a| a == "--uninstall-ui");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon({
                let icon = clipcraft_engine::brand::window_icon();
                egui::IconData {
                    width: icon.width(),
                    height: icon.height(),
                    rgba: icon.into_raw(),
                }
            })
            .with_inner_size([700.0, 560.0])
            .with_min_inner_size([620.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        if removing {
            "Uninstall ClipCraft"
        } else {
            "Install ClipCraft"
        },
        options,
        Box::new(move |_creation| {
            Ok(Box::new(Installer {
                destination: destination.to_string_lossy().into_owned(),
                removing,
                accepted: false,
                desktop: false,
                pending: None,
                done: None,
                error: None,
            }))
        }),
    )
    .map_err(|e| e.to_string())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ClipCraft setup: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

struct Installer {
    destination: String,
    removing: bool,
    accepted: bool,
    desktop: bool,
    pending: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    done: Option<PathBuf>,
    error: Option<String>,
}

impl eframe::App for Installer {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(receiver) = self.pending.as_ref() {
            match receiver.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    match result {
                        Ok(path) => self.done = Some(path),
                        Err(error) => self.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error = Some("The installation worker stopped. Retry; existing clips and settings are kept.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    context.request_repaint_after(std::time::Duration::from_millis(100))
                }
            }
        }
        if self.pending.is_some() && context.input(|i| i.viewport().close_requested()) {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(egui::Color32::from_rgb(18,18,21)).inner_margin(28.0)).show(context, |ui| {
            ui.visuals_mut().override_text_color = Some(egui::Color32::from_gray(230));
            egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(if self.removing { "Uninstall ClipCraft" } else { "ClipCraft · CUT" });
            ui.label(format!("Version {} · Offline · Installs for your Windows account", env!("CARGO_PKG_VERSION")));
            ui.add_space(18.0);
            if self.done.is_some() {
                ui.heading(if self.removing { "ClipCraft has been removed" } else { "Ready to clip" });
                ui.label("Your clips and settings stay on this computer.");
                if !self.removing && ui.button("Launch ClipCraft").clicked()
                    && let Some(path) = &self.done {
                    match std::process::Command::new(path).spawn() { Ok(_) => context.send_viewport_cmd(egui::ViewportCommand::Close), Err(e) => self.error = Some(e.to_string()) }
                }
                if ui.button("Close").clicked() { context.send_viewport_cmd(egui::ViewportCommand::Close); }
            } else {
                ui.add_enabled_ui(self.pending.is_none(), |ui| {
                    ui.label("Application folder");
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 100.0).max(160.0);
                        ui.add_enabled(!self.removing, egui::TextEdit::singleline(&mut self.destination).desired_width(width));
                        if !self.removing && ui.button("Browse…").clicked()
                            && let Some(folder) = rfd::FileDialog::new().pick_folder() { self.destination = folder.to_string_lossy().into_owned(); }
                    });
                    ui.add_space(12.0);
                    if self.removing {
                        ui.label("This removes installed program files and Windows shortcuts. Recordings, library metadata and preferences are kept. Close ClipCraft first.");
                        ui.checkbox(&mut self.accepted, "Remove ClipCraft from this computer");
                    } else {
                        ui.label("No administrator access, account or download is required. The Start menu shortcut and Windows uninstall entry are included.");
                        ui.checkbox(&mut self.desktop, "Add a desktop shortcut");
                        egui::CollapsingHeader::new("License (MIT)").show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| { ui.label(include_str!("../../../LICENSE-MIT")); });
                        });
                        ui.checkbox(&mut self.accepted, "I accept the license");
                    }
                    ui.add_space(18.0);
                    if ui.add_enabled(self.accepted, egui::Button::new(if self.removing { "Uninstall" } else { "Install ClipCraft" }).min_size(egui::vec2(160.0,38.0))).clicked() {
                        self.error = None;
                        let destination = PathBuf::from(&self.destination); let removing = self.removing; let desktop = self.desktop;
                        let (sender, receiver) = mpsc::channel(); let wake = context.clone();
                        match std::thread::Builder::new().name("clipcraft-installer".into()).spawn(move || {
                            let result = if removing { uninstall(destination, true) } else { install(destination, true, desktop) };
                            let _ = sender.send(result); wake.request_repaint();
                        }) {
                            Ok(_) => self.pending = Some(receiver), Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                });
                if self.pending.is_some() { ui.spinner(); ui.label("Working… Keep this window open."); }
            }
            if let Some(error) = &self.error { ui.colored_label(egui::Color32::from_rgb(255,130,130), error); }
            });
        });
    }
}
