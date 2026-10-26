//! The system-tray icon: closing the window can minimize ClipCraft instead of quitting, and the
//! icon brings it back. A left click opens the window; a right click asks for the app's own menu
//! (`tray_menu`), which is drawn by ClipCraft rather than by the operating system. Windows only;
//! other platforms have no tray and close normally.

/// What the tray asks the window to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Show the tray menu at this screen position, in physical pixels.
    Menu {
        x: f32,
        y: f32,
    },
    Open,
    ToggleRecording,
    SaveClip,
    Screenshot,
    OpenClipsFolder,
    OpenLibrary,
    OpenSettings,
    Quit,
}

#[cfg(windows)]
mod imp {
    use super::Action;
    use eframe::egui;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    /// Set when the person chose Quit, so the close that follows is not turned into a minimize.
    static QUITTING: AtomicBool = AtomicBool::new(false);

    pub struct Tray {
        _icon: TrayIcon,
        actions: mpsc::Receiver<Action>,
    }

    pub fn show(context: &egui::Context) {
        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        context.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        context.send_viewport_cmd(egui::ViewportCommand::Focus);
        context.request_repaint();
    }

    impl Tray {
        /// `None` when the icon cannot be created; the window then closes normally.
        pub fn new(context: &egui::Context) -> Option<Self> {
            let image = clipcraft_engine::brand::window_icon();
            let (width, height) = (image.width(), image.height());
            let icon = tray_icon::Icon::from_rgba(image.into_raw(), width, height).ok()?;
            let (sender, actions) = mpsc::channel();
            let handle = context.clone();
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button,
                    button_state: MouseButtonState::Up,
                    position,
                    ..
                } = event
                {
                    match button {
                        MouseButton::Left => show(&handle),
                        MouseButton::Right => {
                            let _ = sender.send(Action::Menu {
                                x: position.x as f32,
                                y: position.y as f32,
                            });
                            handle.request_repaint();
                        }
                        MouseButton::Middle => {}
                    }
                }
            }));
            let icon = TrayIconBuilder::new()
                .with_tooltip("ClipCraft")
                .with_icon(icon)
                .build()
                .ok()?;
            Some(Self {
                _icon: icon,
                actions,
            })
        }

        pub fn next_action(&self) -> Option<Action> {
            self.actions.try_recv().ok()
        }

        /// Closes the window for good, past the close-to-tray minimize.
        pub fn quit(context: &egui::Context) {
            QUITTING.store(true, Ordering::SeqCst);
            show(context);
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        pub fn quitting() -> bool {
            QUITTING.load(Ordering::SeqCst)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use eframe::egui;

    pub struct Tray;

    pub fn show(_context: &egui::Context) {}

    impl Tray {
        pub fn new(_context: &egui::Context) -> Option<Self> {
            None
        }

        pub fn next_action(&self) -> Option<super::Action> {
            None
        }

        pub fn quit(context: &egui::Context) {
            context.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        pub fn quitting() -> bool {
            false
        }
    }
}

pub use imp::{Tray, show};
