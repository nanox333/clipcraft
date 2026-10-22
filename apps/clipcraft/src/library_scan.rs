//! A replaceable traversal worker. Only the UI thread writes library metadata.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use clipcraft_engine::{Command, CommandOutput, EngineError, LibraryScanResult, execute};
use eframe::egui;

use crate::ClipCraftApp;

pub(crate) struct ScanJob {
    directory: String,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Result<LibraryScanResult, EngineError>>,
}

impl Drop for ScanJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct Request {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    sender: mpsc::SyncSender<Result<LibraryScanResult, EngineError>>,
}

#[derive(Default)]
struct Mailbox {
    pending: Option<Request>,
    stopped: bool,
}

/// One traversal thread and one replaceable pending request per app, including during slow I/O.
pub(crate) struct ScanWorker {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    thread: std::thread::JoinHandle<()>,
}

impl ScanWorker {
    fn start() -> Result<Self, ()> {
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let worker_mailbox = Arc::clone(&mailbox);
        let thread = std::thread::Builder::new()
            .name("library-scan".into())
            .spawn(move || {
                loop {
                    let (lock, ready) = &*worker_mailbox;
                    let Ok(mut state) = lock.lock() else {
                        return;
                    };
                    while state.pending.is_none() && !state.stopped {
                        state = match ready.wait(state) {
                            Ok(state) => state,
                            Err(_) => return,
                        };
                    }
                    if state.stopped {
                        return;
                    }
                    let request = state.pending.take();
                    drop(state);
                    if let Some(request) = request {
                        let result = clipcraft_engine::execute_cancellable(
                            Command::ScanLibrary {
                                directory: request.path,
                            },
                            &request.cancel,
                        )
                        .and_then(|output| match output {
                            CommandOutput::LibraryScan(scan) => Ok(scan),
                            _ => Err(EngineError::IndexInvalid),
                        });
                        let _ = request.sender.send(result);
                    }
                }
            })
            .map_err(|_| ())?;
        Ok(Self { mailbox, thread })
    }

    fn submit(&self, request: Request) -> Result<(), ()> {
        let (lock, ready) = &*self.mailbox;
        let mut state = lock.lock().map_err(|_| ())?;
        state.pending = Some(request);
        ready.notify_one();
        Ok(())
    }
}

impl Drop for ScanWorker {
    fn drop(&mut self) {
        let (lock, ready) = &*self.mailbox;
        if let Ok(mut state) = lock.lock() {
            state.stopped = true;
            state.pending = None;
            ready.notify_one();
        }
    }
}

impl ClipCraftApp {
    pub(crate) fn scan_library(&mut self) {
        // Dropping a replaced worker cancels traversal and prevents stale results being applied.
        self.scanning = None;
        self.error = None;
        let directory = self.directory_text.trim().to_owned();
        let path = PathBuf::from(&directory);
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(1);
        if self
            .scan_worker
            .as_ref()
            .is_none_or(|worker| worker.thread.is_finished())
        {
            self.scan_worker = ScanWorker::start().ok();
        }
        let started = self.scan_worker.as_ref().ok_or(()).and_then(|worker| {
            worker.submit(Request {
                path,
                cancel: Arc::clone(&cancel),
                sender,
            })
        });
        match started {
            Ok(_) => {
                self.scanning = Some(ScanJob {
                    directory,
                    cancel,
                    receiver,
                })
            }
            Err(_) => self.error = Some("The library scan could not start. Try again.".into()),
        }
    }

    pub(crate) fn poll_library_scan(&mut self, context: &egui::Context) {
        let Some(job) = self.scanning.as_ref() else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => {
                context.request_repaint_after(std::time::Duration::from_millis(50));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(EngineError::DirectoryUnavailable),
        };
        let Some(job) = self.scanning.take() else {
            return;
        };
        if job.directory != self.directory_text.trim() {
            return;
        }
        let result = result.and_then(|mut scan| {
            if let Some(index_path) = self.index_path.clone() {
                scan = match execute(Command::StoreLibraryScan { index_path, scan })? {
                    CommandOutput::LibraryScan(scan) => scan,
                    _ => return Err(EngineError::IndexSaveFailed),
                };
            }
            if !scan.persisted
                && let Some(previous) = self
                    .library
                    .as_ref()
                    .filter(|previous| previous.directory == scan.directory)
            {
                scan.entries =
                    clipcraft_engine::merge_library_entries(Some(&previous.entries), scan.entries);
                scan.collections = previous.collections.clone();
            }
            // In-memory fallback if no index is configured, or persistence was unavailable.
            for entry in &mut scan.entries {
                if entry.recorded_game.is_none() {
                    entry.recorded_game = self.saved_games.get(&entry.path).cloned();
                }
            }
            Ok(scan)
        });
        match result {
            Ok(result) => {
                if self.index_path.is_some() && !result.persisted {
                    self.error = Some(EngineError::IndexSaveFailed.to_string());
                }
                if self
                    .library
                    .as_ref()
                    .is_none_or(|old| old.directory != result.directory)
                {
                    self.probe_attempted.clear();
                    self.thumbnail_attempted.clear();
                    self.probe_generation = self.probe_generation.wrapping_add(1);
                    self.probe_receiver = None;
                    self.thumbnail_receiver = None;
                    self.unsaved_probes.clear();
                }
                self.directory_text = result.directory.clone();
                self.library = Some(result);
                self.needs_probe = true;
                self.needs_thumbnails = true;
            }
            Err(EngineError::ScanCancelled) => {}
            Err(error) => {
                if error == EngineError::DirectoryUnavailable
                    && let Some(library) = self
                        .library
                        .as_mut()
                        .filter(|library| library.directory == job.directory)
                {
                    for entry in &mut library.entries {
                        entry.offline = !std::path::Path::new(&entry.path).exists();
                    }
                }
                self.error = Some(error.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_scan_is_named_and_reachable_with_tab_and_enter() {
        let context = crate::kit::test_context();
        context.enable_accesskit();
        let (_sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut app = ClipCraftApp {
            directory_text: "/clips".into(),
            library: Some(LibraryScanResult {
                directory: "/clips".into(),
                entries: Vec::new(),
                persisted: false,
                collections: Vec::new(),
            }),
            scanning: Some(ScanJob {
                directory: "/clips".into(),
                cancel: Arc::clone(&cancel),
                receiver,
            }),
            ..Default::default()
        };
        let old = app.library.clone();
        let mut names = std::collections::HashMap::new();
        let mut reached = false;
        for frame in 0..100 {
            let key = if reached {
                egui::Key::Enter
            } else {
                egui::Key::Tab
            };
            let events = if frame < 3 {
                Vec::new()
            } else {
                vec![
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| {
                    crate::kit::apply_theme(context);
                    app.draw_screen(context);
                },
            );
            if reached {
                break;
            }
            if let Some(update) = output.platform_output.accesskit_update {
                for (id, node) in update.nodes {
                    names.insert(id, node.label().unwrap_or_default().to_owned());
                }
                reached = names
                    .get(&update.focus)
                    .is_some_and(|label| label == "Cancel scan");
            }
        }
        assert!(reached, "Cancel scan was not reachable by Tab");
        assert!(app.scanning.is_none(), "Enter did not activate Cancel scan");
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(app.library, old);
    }

    fn finish(app: &mut ClipCraftApp) {
        let context = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.scanning.is_some() {
            assert!(std::time::Instant::now() < deadline);
            app.poll_library_scan(&context);
            std::thread::yield_now();
        }
    }

    #[test]
    fn replacement_uses_one_worker_and_cancel_keeps_the_existing_library() {
        let root = crate::tests::temporary_directory();
        std::fs::create_dir(root.join("Hades")).unwrap();
        std::fs::write(root.join("Hades/clip.mp4"), b"fixture").unwrap();
        let mut app = ClipCraftApp {
            directory_text: root.to_str().unwrap().into(),
            ..Default::default()
        };
        app.scan_library();
        let worker_id = app.scan_worker.as_ref().unwrap().thread.thread().id();
        let cancel = Arc::clone(&app.scanning.as_ref().unwrap().cancel);
        for _ in 0..50 {
            app.scan_library();
        }
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(
            worker_id,
            app.scan_worker.as_ref().unwrap().thread.thread().id()
        );
        finish(&mut app);
        assert!(app.error.is_none());
        assert_eq!(app.library.as_ref().unwrap().entries.len(), 1);
        let old = app.library.clone();
        app.scan_library();
        let cancel = Arc::clone(&app.scanning.as_ref().unwrap().cancel);
        app.scanning = None;
        app.poll_library_scan(&egui::Context::default());
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(app.library, old);
        app.directory_text = root.join("missing").to_str().unwrap().into();
        app.scan_library();
        finish(&mut app);
        assert!(app.error.is_some());
        assert_eq!(app.library, old);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn metadata_edits_survive_a_background_rescan_without_an_index() {
        let root = crate::tests::temporary_directory();
        std::fs::write(root.join("clip.mp4"), b"fixture").unwrap();
        let mut app = ClipCraftApp {
            directory_text: root.to_str().unwrap().into(),
            ..Default::default()
        };
        app.scan_library();
        finish(&mut app);
        app.scan_library();
        let entry = &mut app.library.as_mut().unwrap().entries[0];
        entry.favorite = true;
        entry.game = Some("Manual".into());
        finish(&mut app);
        let entry = &app.library.as_ref().unwrap().entries[0];
        assert!(entry.favorite);
        assert_eq!(entry.game.as_deref(), Some("Manual"));
        let _ = std::fs::remove_dir_all(root);
    }
}
