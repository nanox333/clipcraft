#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Native offline installer. GUI and explicit unattended arguments share engine commands.
use clipcraft_engine::{Command, CommandOutput, execute};
use eframe::egui::{self, Color32, RichText};
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
    } else if arguments.is_empty() || (cfg!(debug_assertions) && preview(&arguments).is_some()) {
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
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size([740.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        if removing {
            "Uninstall ClipCraft"
        } else {
            "Install ClipCraft"
        },
        options,
        Box::new(move |creation| {
            apply_style(&creation.egui_ctx);
            let mut installer =
                Installer::new(destination.to_string_lossy().into_owned(), removing);
            #[cfg(debug_assertions)]
            if let Some(page) = preview(&std::env::args_os().skip(1).collect::<Vec<_>>()) {
                installer.preview(page);
            }
            Ok(Box::new(installer))
        }),
    )
    .map_err(|e| e.to_string())
}

/// Development builds only: `--preview working|done` shows those pages without installing anything,
/// so their look can be checked.
fn preview(arguments: &[std::ffi::OsString]) -> Option<&'static str> {
    match arguments.first()?.to_str()? {
        "--preview-working" => Some("working"),
        "--preview-done" => Some("done"),
        _ => None,
    }
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

const CANVAS: Color32 = Color32::from_rgb(17, 17, 19);
const SIDEBAR: Color32 = Color32::from_rgb(22, 22, 25);
const CARD: Color32 = Color32::from_rgb(32, 32, 37);
const BORDER: Color32 = Color32::from_rgb(44, 44, 50);
const CONTROL: Color32 = Color32::from_rgb(46, 46, 53);
const CONTROL_HOVER: Color32 = Color32::from_rgb(58, 58, 66);
const TEXT: Color32 = Color32::from_rgb(240, 240, 244);
const MUTED: Color32 = Color32::from_rgb(150, 150, 161);
const LIME: Color32 = Color32::from_rgb(190, 255, 43);
const DANGER: Color32 = Color32::from_rgb(255, 112, 112);

fn semibold() -> egui::FontFamily {
    egui::FontFamily::Name("semibold".into())
}

fn medium() -> egui::FontFamily {
    egui::FontFamily::Name("medium".into())
}

/// The ClipCraft look: Inter, charcoal surfaces and lime accents, as in the app.
fn apply_style(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "inter",
            &include_bytes!("../../../assets/fonts/Inter-Regular.ttf")[..],
        ),
        (
            "inter-medium",
            &include_bytes!("../../../assets/fonts/Inter-Medium.ttf")[..],
        ),
        (
            "inter-semibold",
            &include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf")[..],
        ),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    let fallbacks = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, face) in [
        (egui::FontFamily::Proportional, "inter"),
        (egui::FontFamily::Name("medium".into()), "inter-medium"),
        (egui::FontFamily::Name("semibold".into()), "inter-semibold"),
    ] {
        let mut chain = vec![face.to_owned()];
        chain.extend(fallbacks.iter().cloned());
        fonts.families.insert(family, chain);
    }
    context.set_fonts(fonts);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = CANVAS;
    visuals.override_text_color = Some(TEXT);
    context.set_visuals(visuals);
}

/// A rounded text button: lime when it is the way forward, red when it removes, quiet otherwise.
fn pill_button(
    ui: &mut egui::Ui,
    label: &str,
    kind: Kind,
    enabled: bool,
    min_width: f32,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::new(14.5, medium()),
        Color32::PLACEHOLDER,
    );
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2((galley.size().x + 48.0).max(min_width), 46.0),
        sense,
    );
    let hovered = enabled && response.hovered();
    let (fill, ink) = match (kind, hovered) {
        (Kind::Primary, false) => (LIME, Color32::BLACK),
        (Kind::Primary, true) => (Color32::from_rgb(210, 255, 90), Color32::BLACK),
        (Kind::Danger, false) => (DANGER, Color32::BLACK),
        (Kind::Danger, true) => (Color32::from_rgb(255, 140, 140), Color32::BLACK),
        (Kind::Quiet, false) => (CONTROL, TEXT),
        (Kind::Quiet, true) => (CONTROL_HOVER, TEXT),
    };
    ui.painter().rect_filled(rect, 12.0, fill);
    ui.painter().galley(
        egui::pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ink,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy)]
enum Kind {
    Primary,
    Danger,
    Quiet,
}

/// An on/off switch with its label and a line of help.
fn switch_row(ui: &mut egui::Ui, title: &str, help: &str, value: &mut bool) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 56.0).max(0.0));
                    ui.label(
                        RichText::new(title)
                            .size(14.5)
                            .family(semibold())
                            .color(TEXT),
                    );
                    ui.label(RichText::new(help).size(12.5).color(MUTED));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(40.0, 22.0), egui::Sense::click());
                    let on = *value;
                    ui.painter().rect_filled(
                        rect,
                        11.0,
                        if on {
                            LIME
                        } else {
                            Color32::from_rgb(62, 62, 69)
                        },
                    );
                    ui.painter().circle_filled(
                        egui::pos2(
                            if on {
                                rect.right() - 11.0
                            } else {
                                rect.left() + 11.0
                            },
                            rect.center().y,
                        ),
                        8.0,
                        if on { Color32::BLACK } else { Color32::WHITE },
                    );
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        *value = !*value;
                    }
                });
            });
        });
}

/// A thin bar that slides back and forth while the work is under way.
fn busy_bar(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4.0, CARD);
    let time = ui.input(|input| input.time) as f32;
    let travel = (time * 0.9).fract();
    let width = rect.width() * 0.3;
    let start =
        rect.left() + (rect.width() - width) * (0.5 - 0.5 * (travel * std::f32::consts::TAU).cos());
    ui.painter().rect_filled(
        egui::Rect::from_min_size(egui::pos2(start, rect.top()), egui::vec2(width, 8.0)),
        4.0,
        LIME,
    );
    ui.ctx().request_repaint();
}

struct Installer {
    destination: String,
    removing: bool,
    desktop: bool,
    show_license: bool,
    pending: Option<mpsc::Receiver<Result<PathBuf, String>>>,
    done: Option<PathBuf>,
    error: Option<String>,
    mark: Option<egui::TextureHandle>,
    started: Option<std::time::Instant>,
}

/// A soft lime glow: stacked, fading circles, the nearest thing to a blur that needs no shader.
fn glow(painter: &egui::Painter, centre: egui::Pos2, radius: f32, strength: f32) {
    const RINGS: usize = 64;
    let alpha = (strength * 255.0 / RINGS as f32 * 2.2).clamp(1.0, 255.0) as u8;
    for ring in 0..RINGS {
        let t = ring as f32 / RINGS as f32;
        painter.circle_filled(
            centre,
            radius * (1.0 - t),
            Color32::from_rgba_unmultiplied(190, 255, 43, alpha),
        );
    }
}

/// `C:\Users\you\AppData\Local\Programs\ClipCraft` shortened in the middle so it fits.
fn short_path(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= 46 {
        return path.to_owned();
    }
    let head: String = chars[..16].iter().collect();
    let tail: String = chars[chars.len() - 27..].iter().collect();
    format!("{head}\u{2026}{tail}")
}

/// A small folder drawn with lines, so no icon font is needed.
fn folder_glyph(painter: &egui::Painter, centre: egui::Pos2, ink: Color32) {
    let body = egui::Rect::from_center_size(centre + egui::vec2(0.0, 1.5), egui::vec2(18.0, 12.0));
    painter.rect_stroke(
        body,
        3.0,
        egui::Stroke::new(1.6_f32, ink),
        egui::StrokeKind::Inside,
    );
    painter.line_segment(
        [
            egui::pos2(body.left() + 1.5, body.top() - 1.8),
            egui::pos2(body.left() + 7.0, body.top() - 1.8),
        ],
        egui::Stroke::new(1.8_f32, ink),
    );
}

impl Installer {
    fn new(destination: String, removing: bool) -> Self {
        Self {
            destination,
            removing,
            desktop: false,
            show_license: false,
            pending: None,
            done: None,
            error: None,
            mark: None,
            started: None,
        }
    }

    #[cfg(debug_assertions)]
    fn preview(&mut self, page: &str) {
        if page == "done" {
            self.done = Some(PathBuf::from("clipcraft.exe"));
        } else {
            self.started = Some(std::time::Instant::now());
            // A receiver whose sender is kept alive, so the page stays on screen.
            let (sender, receiver) = mpsc::channel();
            std::mem::forget(sender);
            self.pending = Some(receiver);
        }
    }

    fn mark(&mut self, context: &egui::Context) -> egui::TextureId {
        let texture = self.mark.get_or_insert_with(|| {
            let icon = clipcraft_engine::brand::window_icon();
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width() as usize, icon.height() as usize],
                icon.as_raw(),
            );
            context.load_texture("clipcraft-mark", image, egui::TextureOptions::LINEAR)
        });
        texture.id()
    }

    fn start(&mut self, context: &egui::Context) {
        self.error = None;
        self.started = Some(std::time::Instant::now());
        let destination = PathBuf::from(&self.destination);
        let (removing, desktop) = (self.removing, self.desktop);
        let (sender, receiver) = mpsc::channel();
        let wake = context.clone();
        match std::thread::Builder::new()
            .name("clipcraft-installer".into())
            .spawn(move || {
                let result = if removing {
                    uninstall(destination, true)
                } else {
                    install(destination, true, desktop)
                };
                // The work is quick; showing the progress for a moment lets people see it happen.
                std::thread::sleep(std::time::Duration::from_millis(900));
                let _ = sender.send(result);
                wake.request_repaint();
            }) {
            Ok(_) => self.pending = Some(receiver),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn poll(&mut self, context: &egui::Context) {
        let Some(receiver) = self.pending.as_ref() else {
            return;
        };
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
                self.error = Some(
                    "The installation worker stopped. Retry; existing clips and settings are kept."
                        .into(),
                );
            }
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
    }

    /// The left panel: a glowing mark, the name, and what ClipCraft is about.
    fn brand_panel(&mut self, context: &egui::Context) {
        let mark = self.mark(context);
        egui::SidePanel::left("brand")
            .exact_width(272.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR))
            .show(context, |ui| {
                let panel = ui.max_rect();
                // A faint lime wash from the top, fading into the panel colour.
                let top = egui::Rect::from_min_size(panel.min, egui::vec2(panel.width(), 260.0));
                let mut wash = egui::Mesh::default();
                let clear = Color32::from_rgba_unmultiplied(190, 255, 43, 0);
                let tint = Color32::from_rgba_unmultiplied(190, 255, 43, 26);
                wash.colored_vertex(top.left_top(), tint);
                wash.colored_vertex(top.right_top(), tint);
                wash.colored_vertex(top.right_bottom(), clear);
                wash.colored_vertex(top.left_bottom(), clear);
                wash.add_triangle(0, 1, 2);
                wash.add_triangle(0, 2, 3);
                ui.painter().add(egui::Shape::mesh(wash));
                let centre = egui::pos2(panel.left() + 66.0, panel.top() + 92.0);
                glow(ui.painter(), centre, 90.0, 0.22);
                ui.painter().image(
                    mark,
                    egui::Rect::from_center_size(centre, egui::vec2(72.0, 72.0)),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                let mut text =
                    ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_max(
                        egui::pos2(panel.left() + 30.0, panel.top() + 150.0),
                        egui::pos2(panel.right() - 24.0, panel.bottom() - 20.0),
                    )));
                text.label(
                    RichText::new("ClipCraft")
                        .size(30.0)
                        .family(semibold())
                        .color(TEXT),
                );
                text.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .size(12.5)
                        .color(MUTED),
                );
                text.add_space(22.0);
                for line in [
                    "Saves your last moments",
                    "Stays on your computer",
                    "No account, no uploads",
                ] {
                    text.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                        ui.painter().circle_filled(
                            rect.center(),
                            9.0,
                            Color32::from_rgba_unmultiplied(190, 255, 43, 36),
                        );
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "\u{2713}",
                            egui::FontId::new(11.0, semibold()),
                            LIME,
                        );
                        ui.add_space(4.0);
                        ui.label(RichText::new(line).size(13.0).color(TEXT));
                    });
                    text.add_space(8.0);
                }
            });
    }

    fn title_bar(&self, ui: &mut egui::Ui, context: &egui::Context) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 30.0),
            egui::Sense::click_and_drag(),
        );
        if response.drag_started() {
            context.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let close = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 14.0, rect.center().y),
            egui::vec2(28.0, 28.0),
        );
        let over = ui.rect_contains_pointer(close);
        if over {
            ui.painter().rect_filled(close, 8.0, CONTROL);
        }
        let ink = if over { TEXT } else { MUTED };
        let c = close.center();
        for (a, b) in [
            (egui::vec2(-4.5, -4.5), egui::vec2(4.5, 4.5)),
            (egui::vec2(-4.5, 4.5), egui::vec2(4.5, -4.5)),
        ] {
            ui.painter()
                .line_segment([c + a, c + b], egui::Stroke::new(1.6_f32, ink));
        }
        if over && ui.input(|input| input.pointer.primary_released()) && self.pending.is_none() {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn heading(&self, ui: &mut egui::Ui, label: &str, title: &str, lead: &str) {
        ui.label(
            RichText::new(label)
                .size(11.5)
                .family(semibold())
                .color(LIME),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(title)
                .size(28.0)
                .family(semibold())
                .color(TEXT),
        );
        ui.add_space(6.0);
        ui.label(RichText::new(lead).size(14.0).color(MUTED));
        ui.add_space(22.0);
    }

    fn options_page(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        let (label, title, lead) = if self.removing {
            (
                "UNINSTALL",
                "Remove ClipCraft?",
                "This removes the app and its shortcuts. Your clips, settings and library stay on this computer. Close ClipCraft first.",
            )
        } else {
            (
                "SETUP",
                "Install ClipCraft",
                "It takes a few seconds, adds a Start menu shortcut and an uninstaller, and needs nothing else.",
            )
        };
        self.heading(ui, label, title, lead);
        if !self.removing {
            egui::Frame::new()
                .fill(CARD)
                .stroke(egui::Stroke::new(1.0_f32, BORDER))
                .corner_radius(12.0)
                .inner_margin(egui::Margin::symmetric(14, 11))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                        folder_glyph(ui.painter(), rect.center(), MUTED);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new("Install location").size(12.0).color(MUTED));
                            ui.label(
                                RichText::new(short_path(&self.destination))
                                    .size(13.0)
                                    .family(medium())
                                    .color(TEXT),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let change = ui
                                .add(
                                    egui::Label::new(
                                        RichText::new("Change")
                                            .size(13.0)
                                            .family(medium())
                                            .color(LIME),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if change.clicked()
                                && let Some(folder) = rfd::FileDialog::new()
                                    .set_title("Choose where to install ClipCraft")
                                    .pick_folder()
                            {
                                self.destination =
                                    folder.join("ClipCraft").to_string_lossy().into_owned();
                            }
                        });
                    });
                });
            ui.add_space(10.0);
            switch_row(
                ui,
                "Desktop shortcut",
                "Add a ClipCraft icon to your desktop.",
                &mut self.desktop,
            );
            if self.show_license {
                ui.add_space(10.0);
                egui::Frame::new()
                    .fill(CARD)
                    .corner_radius(10.0)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(70.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(
                                    RichText::new(include_str!("../../../LICENSE-MIT"))
                                        .size(11.5)
                                        .color(MUTED),
                                );
                            });
                    });
            }
        }
        if let Some(error) = &self.error {
            ui.add_space(10.0);
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(255, 112, 112, 24))
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    Color32::from_rgba_unmultiplied(255, 112, 112, 90),
                ))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(error).size(12.5).color(DANGER));
                });
        }
        ui.add_space((ui.available_height() - if self.removing { 52.0 } else { 98.0 }).max(8.0));
        ui.scope(|ui| {
            if !self.removing {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Free and open source under the MIT license.")
                            .size(12.0)
                            .color(MUTED),
                    );
                    let word = if self.show_license { "Hide" } else { "Read it" };
                    if ui
                        .add(
                            egui::Label::new(RichText::new(word).size(12.0).color(LIME))
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.show_license = !self.show_license;
                    }
                });
                ui.add_space(10.0);
            }
            let (text, kind) = if self.removing {
                ("Remove ClipCraft", Kind::Danger)
            } else {
                ("Install ClipCraft", Kind::Primary)
            };
            ui.horizontal(|ui| {
                if self.removing && pill_button(ui, "Cancel", Kind::Quiet, true, 0.0).clicked() {
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let width = ui.available_width();
                if pill_button(ui, text, kind, true, width).clicked() {
                    self.start(context);
                }
            });
        });
    }

    fn working_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(64.0);
        self.heading(
            ui,
            if self.removing {
                "UNINSTALLING"
            } else {
                "INSTALLING"
            },
            if self.removing {
                "Removing ClipCraft\u{2026}"
            } else {
                "Setting things up\u{2026}"
            },
            "Keep this window open for a moment.",
        );
        busy_bar(ui);
        let seconds = self.started.map_or(0.0, |at| at.elapsed().as_secs_f32());
        let step = if self.removing {
            [
                "Closing down",
                "Removing program files",
                "Removing shortcuts",
            ]
        } else {
            [
                "Copying program files",
                "Creating shortcuts",
                "Registering the uninstaller",
            ]
        }[((seconds / 0.3) as usize).min(2)];
        ui.add_space(12.0);
        ui.label(RichText::new(step).size(13.0).color(MUTED));
    }

    fn done_page(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        ui.add_space(26.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(76.0, 76.0), egui::Sense::hover());
        glow(ui.painter(), rect.center(), 80.0, 0.18);
        ui.painter().circle_filled(rect.center(), 34.0, LIME);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "\u{2713}",
            egui::FontId::new(34.0, semibold()),
            Color32::BLACK,
        );
        ui.add_space(20.0);
        self.heading(
            ui,
            if self.removing { "DONE" } else { "ALL SET" },
            if self.removing {
                "ClipCraft has been removed"
            } else {
                "ClipCraft is installed"
            },
            if self.removing {
                "Your clips and settings are still on this computer."
            } else {
                "Find it in your Start menu. Your clips stay on this computer."
            },
        );
        ui.horizontal(|ui| {
            if !self.removing
                && let Some(path) = self.done.clone()
                && pill_button(ui, "Launch ClipCraft", Kind::Primary, true, 0.0).clicked()
            {
                match std::process::Command::new(path).spawn() {
                    Ok(_) => context.send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            if pill_button(ui, "Close", Kind::Quiet, true, 0.0).clicked() {
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        if let Some(error) = &self.error {
            ui.add_space(10.0);
            ui.label(RichText::new(error).size(12.5).color(DANGER));
        }
    }
}

impl eframe::App for Installer {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(context);
        if self.pending.is_some() && context.input(|i| i.viewport().close_requested()) {
            context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.brand_panel(context);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(egui::Margin {
                left: 36,
                right: 24,
                top: 12,
                bottom: 30,
            }))
            .show(context, |ui| {
                self.title_bar(ui, context);
                ui.add_space(8.0);
                if self.done.is_some() {
                    self.done_page(ui, context);
                } else if self.pending.is_some() {
                    self.working_page(ui);
                } else {
                    self.options_page(ui, context);
                }
            });
        // A hairline around the whole window, since it has no frame of its own.
        context
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("installer-border"),
            ))
            .rect_stroke(
                context.content_rect().shrink(0.5),
                0.0,
                egui::Stroke::new(1.0_f32, BORDER),
                egui::StrokeKind::Inside,
            );
    }
}
