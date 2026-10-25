//! One ordered owner of desktop index writes. Failed jobs pause the queue for explicit retry.
use clipcraft_engine::{
    Command, CommandOutput, EngineError, LibraryScanResult, MediaProbe, MetaEdit, execute,
};
use eframe::egui;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
};
const MAX_JOBS: usize = 64;
const MAX_BYTES: usize = 128 * 1024 * 1024;
const SCAN_BYTES: usize = 48 * 1024 * 1024;
const STOPPED: &str =
    "Library saving stopped unexpectedly. Your clips are untouched. Retry saving.";

#[derive(Clone)]
pub(crate) enum Write {
    Scan {
        index: PathBuf,
        scan: LibraryScanResult,
        generation: u64,
    },
    Edit {
        index: PathBuf,
        directory: String,
        edit: MetaEdit,
    },
    Probes {
        index: PathBuf,
        directory: String,
        probes: Vec<(String, MediaProbe)>,
    },
    Saved {
        index: PathBuf,
        directory: PathBuf,
        path: PathBuf,
        game: String,
    },
}
impl Write {
    fn charge(&self) -> usize {
        let payload = match self {
            Self::Scan { .. } => SCAN_BYTES,
            Self::Edit { edit, .. } => 1024_usize.saturating_add(match edit {
                MetaEdit::SetFavorite { path, .. } | MetaEdit::RemoveFromLibrary { path } => {
                    path.capacity()
                }
                MetaEdit::SetGame { path, game } => path
                    .capacity()
                    .saturating_add(game.as_ref().map_or(0, String::capacity)),
                MetaEdit::CreateCollection { name } | MetaEdit::DeleteCollection { name } => {
                    name.capacity()
                }
                MetaEdit::RenameCollection { from, to } | MetaEdit::RelinkFile { from, to } => {
                    from.capacity().saturating_add(to.capacity())
                }
                MetaEdit::AddToCollection { path, name }
                | MetaEdit::RemoveFromCollection { path, name } => {
                    path.capacity().saturating_add(name.capacity())
                }
            }),
            Self::Probes { probes, .. } => 1024_usize.saturating_add(
                probes.iter().fold(
                    probes
                        .capacity()
                        .saturating_mul(std::mem::size_of::<(String, MediaProbe)>()),
                    |n, (p, f)| {
                        n.saturating_add(p.capacity())
                            .saturating_add(f.container.capacity())
                            .saturating_add(f.video_codec.as_ref().map_or(0, String::capacity))
                    },
                ),
            ),
            Self::Saved { game, .. } => 16384_usize.saturating_add(game.capacity()),
        };
        payload.saturating_add(match self {
            Self::Scan { index, scan, .. } => {
                index.capacity().saturating_add(scan.directory.capacity())
            }
            Self::Edit {
                index, directory, ..
            }
            | Self::Probes {
                index, directory, ..
            } => index.capacity().saturating_add(directory.capacity()),
            Self::Saved {
                index,
                directory,
                path,
                ..
            } => index
                .capacity()
                .saturating_add(directory.capacity())
                .saturating_add(path.capacity()),
        })
    }
    fn run(self) -> Result<Option<LibraryScanResult>, EngineError> {
        let scan = matches!(&self, Self::Scan { .. });
        let output = execute(match self {
            Self::Scan { index, scan, .. } => Command::StoreLibraryScan {
                index_path: index,
                scan,
            },
            Self::Edit {
                index,
                directory,
                edit,
            } => Command::EditLibraryMeta {
                index_path: index,
                directory,
                edit,
            },
            Self::Probes {
                index,
                directory,
                probes,
            } => Command::StoreProbes {
                index_path: index,
                directory,
                probes,
            },
            Self::Saved {
                index,
                directory,
                path,
                game,
            } => Command::RegisterSavedClip {
                index_path: index,
                directory,
                path,
                game,
            },
        })?;
        match output {
            CommandOutput::LibraryScan(result) if result.persisted => Ok(scan.then_some(result)),
            _ => Err(EngineError::IndexSaveFailed),
        }
    }
}
struct Job {
    id: u64,
    write: Write,
    bytes: usize,
}
pub(crate) enum Event {
    Done {
        id: u64,
        generation: Option<u64>,
        scan: Option<LibraryScanResult>,
        bytes: usize,
    },
    Failed(String),
    Fallback {
        generation: u64,
        scan: LibraryScanResult,
    },
}
#[derive(Default)]
struct Queue {
    pending: VecDeque<Job>,
    replies: VecDeque<Event>,
    count: usize,
    bytes: usize,
    next: u64,
    blocked: bool,
    stop: bool,
    discard: bool,
}
pub(crate) struct Worker {
    shared: Arc<(Mutex<Queue>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    fn start(wake: egui::Context) -> Result<Self, String> {
        Self::start_with(wake, Write::run)
    }
    fn start_with(
        wake: egui::Context,
        mut run: impl FnMut(Write) -> Result<Option<LibraryScanResult>, EngineError> + Send + 'static,
    ) -> Result<Self, String> {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let task = shared.clone();
        let thread = std::thread::Builder::new()
            .name("library-store".into())
            .spawn(move || loop {
                let (lock, ready) = &*task;
                let Ok(mut q) = lock.lock() else { return; };
                while (q.pending.is_empty() || q.blocked) && !q.stop {
                    q = match ready.wait(q) { Ok(q) => q, Err(_) => return };
                }
                if q.discard || (q.stop && (q.pending.is_empty() || q.blocked)) { return; }
                let Some(job) = q.pending.pop_front() else { continue; };
                drop(q);
                let generation = match &job.write { Write::Scan {generation, ..} => Some(*generation), _ => None };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(job.write.clone())))
                    .unwrap_or(Err(EngineError::IndexSaveFailed));
                let Ok(mut q) = lock.lock() else { return; };
                match result {
                    Ok(scan) => q.replies.push_back(Event::Done {id:job.id, generation, scan, bytes:job.bytes}),
                    Err(error) => {
                        q.blocked = true;
                        q.pending.push_front(job);
                        q.replies.retain(|event| matches!(event, Event::Done {..}));
                        let fallback = q.pending.iter().rev().find_map(|job| match &job.write {
                            Write::Scan {scan, generation, ..} => Some((*generation, scan.clone())), _ => None
                        });
                        if let Some((generation, scan)) = fallback { q.replies.push_back(Event::Fallback {generation, scan}); }
                        q.replies.push_back(Event::Failed(format!("Library changes have not been saved: {error} Retry saving before closing.")));
                    }
                }
                drop(q);
                wake.request_repaint();
            })
            .map_err(|_| STOPPED.to_owned())?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }
    pub fn submit(&self, write: Write) -> Result<u64, String> {
        let bytes = write.charge();
        let (lock, ready) = &*self.shared;
        let mut q = lock.lock().map_err(|_| STOPPED.to_owned())?;
        if q.stop || self.thread.as_ref().is_none_or(|t| t.is_finished()) {
            return Err(STOPPED.into());
        }
        if q.count >= MAX_JOBS || bytes > MAX_BYTES.saturating_sub(q.bytes) {
            return Err(
                "Library saving is busy. Wait for pending changes or retry saving, then try again."
                    .into(),
            );
        }
        let id = q.next.checked_add(1).ok_or_else(|| STOPPED.to_owned())?;
        q.next = id;
        if q.blocked
            && let Write::Scan {
                scan, generation, ..
            } = &write
        {
            let fallback = Event::Fallback {
                generation: *generation,
                scan: scan.clone(),
            };
            q.replies.retain(|e| !matches!(e, Event::Fallback { .. }));
            q.replies.push_back(fallback);
        }
        q.pending.push_back(Job { id, write, bytes });
        q.count += 1;
        q.bytes += bytes;
        ready.notify_one();
        Ok(id)
    }
    fn poll(&self) -> Result<Vec<Event>, String> {
        let mut q = self.shared.0.lock().map_err(|_| STOPPED.to_owned())?;
        if self.thread.as_ref().is_none_or(|t| t.is_finished()) && q.count > 0 {
            return Err(STOPPED.into());
        }
        let events: Vec<_> = q.replies.drain(..).collect();
        for event in &events {
            if let Event::Done { bytes, .. } = event {
                q.count = q.count.saturating_sub(1);
                q.bytes = q.bytes.saturating_sub(*bytes);
            }
        }
        Ok(events)
    }
    fn pending(&self) -> usize {
        self.shared.0.lock().map_or(1, |q| q.count)
    }
    fn retry(&self) {
        if let Ok(mut q) = self.shared.0.lock() {
            q.blocked = false;
            self.shared.1.notify_one();
        }
    }
    fn discard(&self) {
        let mut q = self
            .shared
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        q.discard = true;
        q.stop = true;
        q.pending.clear();
        q.replies.clear();
        q.count = 0;
        q.bytes = 0;
        self.shared.1.notify_all();
    }
    fn join(&mut self) -> Result<(), String> {
        {
            let mut q = self
                .shared
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            q.stop = true;
            self.shared.1.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| STOPPED.to_owned())?;
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Err(error) = self.join()
            && let Ok(mut q) = self.shared.0.lock()
        {
            q.replies.push_back(Event::Failed(error));
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    worker: Option<Worker>,
    pub wake: egui::Context,
    pub generation: u64,
    scans: Vec<u64>,
    edits: Vec<(u64, String, MetaEdit)>,
    pub error: Option<String>,
    since: Option<std::time::Instant>,
    pub closing: bool,
}
impl State {
    pub fn submit(&mut self, write: Write) -> Result<u64, String> {
        if self.worker.is_none() {
            self.worker = Some(Worker::start(self.wake.clone())?);
        }
        let is_scan = matches!(&write, Write::Scan { .. });
        let edit = match &write {
            Write::Edit {
                directory, edit, ..
            } if !self.scans.is_empty() => Some((directory.clone(), edit.clone())),
            _ => None,
        };
        let id = self
            .worker
            .as_ref()
            .ok_or_else(|| STOPPED.to_owned())?
            .submit(write)?;
        if is_scan {
            self.scans.push(id);
        }
        if let Some((directory, edit)) = edit {
            self.edits.push((id, directory, edit));
        }
        self.since.get_or_insert_with(std::time::Instant::now);
        Ok(id)
    }
    pub fn poll(&mut self) -> Vec<Event> {
        let events = match self.worker.as_ref().map(Worker::poll).transpose() {
            Ok(events) => events.unwrap_or_default(),
            Err(error) => {
                self.error = Some(error);
                Vec::new()
            }
        };
        for event in &events {
            if let Event::Failed(error) = event {
                self.error = Some(error.clone());
            }
        }
        if self.pending() == 0 {
            self.since = None;
        }
        events
    }
    pub fn rebase(&mut self, id: u64, scan: &mut LibraryScanResult) -> Result<(), EngineError> {
        for (after, directory, edit) in &self.edits {
            let global = matches!(
                edit,
                MetaEdit::CreateCollection { .. }
                    | MetaEdit::RenameCollection { .. }
                    | MetaEdit::DeleteCollection { .. }
            );
            if *after > id && (directory == &scan.directory || global) {
                clipcraft_engine::apply_edit(&mut scan.entries, &mut scan.collections, edit)?;
            }
        }
        Ok(())
    }
    pub fn completed(&mut self, id: u64) {
        self.scans.retain(|&scan| scan != id);
        if let Some(&first) = self.scans.iter().min() {
            self.edits.retain(|(id, _, _)| *id > first);
        } else {
            self.edits.clear();
        }
    }
    pub fn pending(&self) -> usize {
        self.worker.as_ref().map_or(0, Worker::pending)
    }
    pub fn retry(&mut self) {
        if let Some(worker) = &self.worker {
            worker.retry();
        }
        self.error = None;
    }
    pub fn discard(&mut self) {
        if let Some(worker) = &self.worker {
            worker.discard();
        }
        self.worker = None;
        self.scans.clear();
        self.edits.clear();
        self.error = None;
    }
    pub fn visible(&self) -> bool {
        self.error.is_some()
            || self.closing
            || self.since.is_some_and(|at| at.elapsed().as_millis() > 150)
    }
    pub fn finish(&mut self) {
        if let Some(mut worker) = self.worker.take()
            && let Err(error) = worker.join()
        {
            self.error = Some(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::time::{Duration, Instant};
    fn edit(value: bool) -> Write {
        Write::Edit {
            index: "index.json".into(),
            directory: "/clips".into(),
            edit: MetaEdit::SetFavorite {
                path: "/clips/clip.mp4".into(),
                favorite: value,
            },
        }
    }
    fn events_until(worker: &Worker, mut done: impl FnMut(&[Event]) -> bool) -> Vec<Event> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        while !done(&events) {
            assert!(Instant::now() < deadline, "writer did not complete");
            events.extend(worker.poll().unwrap());
            std::thread::sleep(Duration::from_millis(1));
        }
        events
    }
    #[test]
    fn delayed_write_does_not_hold_admission_or_poll_and_budgets_include_inflight_and_replies() {
        let (entered, ready) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let worker = Worker::start_with(egui::Context::default(), move |_| {
            let _ = entered.try_send(());
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(None)
        })
        .unwrap();
        worker.submit(edit(true)).unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 1..MAX_JOBS {
            worker.submit(edit(false)).unwrap();
        }
        assert!(worker.submit(edit(true)).is_err());
        assert!(worker.poll().unwrap().is_empty());
        assert_eq!(worker.pending(), MAX_JOBS);
        for _ in 0..MAX_JOBS {
            release.send(()).unwrap();
        }
        events_until(&worker, |e| e.len() == MAX_JOBS);
        assert_eq!(worker.pending(), 0);
        let mut q = worker.shared.0.lock().unwrap();
        q.bytes = MAX_BYTES;
        drop(q);
        assert!(worker.submit(edit(true)).is_err());
        let mut q = worker.shared.0.lock().unwrap();
        q.bytes = 0;
        q.next = u64::MAX;
        drop(q);
        assert!(worker.submit(edit(true)).is_err());
    }
    #[test]
    fn failure_and_panic_pause_in_order_and_retry_then_shutdown_preserve_sources_and_edits() {
        for panic in [false, true] {
            let root = crate::tests::temporary_directory();
            let clips = root.join("clips");
            std::fs::create_dir(&clips).unwrap();
            let path = clips.join("clip.mp4");
            std::fs::write(&path, b"source unchanged").unwrap();
            let index = root.join("index.json");
            let CommandOutput::LibraryScan(scan) = execute(Command::ScanLibrary {
                directory: clips.clone(),
            })
            .unwrap() else {
                panic!("scan");
            };
            let fail = Arc::new(AtomicBool::new(true));
            let flag = fail.clone();
            let worker = Worker::start_with(egui::Context::default(), move |write| {
                if matches!(&write, Write::Edit { .. }) && flag.swap(false, Ordering::SeqCst) {
                    if panic {
                        panic!("injected writer panic");
                    }
                    return Err(EngineError::IndexSaveFailed);
                }
                write.run()
            })
            .unwrap();
            worker
                .submit(Write::Scan {
                    index: index.clone(),
                    scan,
                    generation: 1,
                })
                .unwrap();
            for favorite in [true, false] {
                worker
                    .submit(Write::Edit {
                        index: index.clone(),
                        directory: clips.to_string_lossy().into_owned(),
                        edit: MetaEdit::SetFavorite {
                            path: path.to_string_lossy().into_owned(),
                            favorite,
                        },
                    })
                    .unwrap();
            }
            events_until(&worker, |events| {
                events.iter().any(|e| matches!(e, Event::Failed(_)))
            });
            assert_eq!(worker.pending(), 2);
            let CommandOutput::LibraryScan(before) = execute(Command::LoadLibrary {
                index_path: index.clone(),
            })
            .unwrap() else {
                panic!("load");
            };
            assert!(!before.entries[0].favorite);
            worker.retry();
            // Drop drains accepted writes, including the failed first edit before the newer one.
            drop(worker);
            let CommandOutput::LibraryScan(after) =
                execute(Command::LoadLibrary { index_path: index }).unwrap()
            else {
                panic!("load");
            };
            assert!(
                !after.entries[0].favorite,
                "retry must not overtake a newer edit"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"source unchanged");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn stale_scan_rebases_newer_local_and_global_edits_and_releases_the_journal() {
        let mut scan = LibraryScanResult {
            directory: "/clips".into(),
            entries: vec![crate::tests::media_entry("clip.mp4", None)],
            persisted: true,
            collections: vec!["Before".into()],
        };
        let mut state = State {
            scans: vec![10, 15],
            edits: vec![
                (
                    11,
                    "/clips".into(),
                    MetaEdit::SetFavorite {
                        path: "/clips/clip.mp4".into(),
                        favorite: true,
                    },
                ),
                (
                    16,
                    "/other".into(),
                    MetaEdit::RenameCollection {
                        from: "Before".into(),
                        to: "After".into(),
                    },
                ),
            ],
            ..Default::default()
        };
        state.rebase(10, &mut scan).unwrap();
        assert!(scan.entries[0].favorite);
        assert_eq!(scan.collections, ["After"]);
        state.completed(10);
        assert_eq!(state.edits.len(), 1);
        state.completed(15);
        assert!(state.edits.is_empty());
    }
    #[test]
    fn poisoned_queue_fails_closed_and_drop_wakes_a_waiting_worker() {
        let worker = Worker::start(egui::Context::default()).unwrap();
        let state = worker.shared.clone();
        let _ = std::thread::spawn(move || {
            let _guard = state.0.lock().unwrap();
            panic!("injected queue poison");
        })
        .join();
        assert!(worker.submit(edit(true)).is_err());
        assert!(worker.poll().is_err());
        drop(worker);
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    #[test]
    fn a_delayed_scan_cannot_undo_newer_ui_edits_and_all_writer_kinds_survive_reload() {
        let root = crate::tests::temporary_directory();
        let clips = root.join("clips");
        std::fs::create_dir(&clips).unwrap();
        let path = clips.join("clip.mp4");
        std::fs::write(&path, b"unchanged source").unwrap();
        let index = root.join("index.json");
        let CommandOutput::LibraryScan(initial) = execute(Command::RefreshLibrary {
            directory: clips.clone(),
            index_path: index.clone(),
        })
        .unwrap() else {
            panic!("initial");
        };
        let CommandOutput::LibraryScan(fresh) = execute(Command::ScanLibrary {
            directory: clips.clone(),
        })
        .unwrap() else {
            panic!("scan");
        };
        let (entered, ready) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let worker = Worker::start_with(egui::Context::default(), move |write| {
            let scan = matches!(&write, Write::Scan { .. });
            let result = write.run();
            if scan {
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            result
        })
        .unwrap();
        let mut app = crate::ClipCraftApp {
            directory_text: clips.to_string_lossy().into_owned(),
            index_path: Some(index.clone()),
            library: Some(initial),
            index_store: State {
                worker: Some(worker),
                ..Default::default()
            },
            ..Default::default()
        };
        app.index_store
            .submit(Write::Scan {
                index: index.clone(),
                scan: fresh,
                generation: 0,
            })
            .unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        for edit in [
            MetaEdit::SetFavorite {
                path: path.to_string_lossy().into_owned(),
                favorite: true,
            },
            MetaEdit::CreateCollection {
                name: "Best".into(),
            },
            MetaEdit::AddToCollection {
                path: path.to_string_lossy().into_owned(),
                name: "Best".into(),
            },
        ] {
            app.edit_library(edit);
            assert!(app.error.is_none());
        }
        let probe = MediaProbe {
            container: "mp4".into(),
            duration: Some(clipcraft_time::Tick::from_units(1, 1)),
            width: Some(64),
            height: Some(48),
            frame_rate: None,
            video_codec: Some("avc1".into()),
            audio_tracks: 0,
        };
        app.unsaved_probes
            .push((path.to_string_lossy().into_owned(), probe.clone()));
        app.save_probes();
        app.index_store
            .submit(Write::Saved {
                index: index.clone(),
                directory: clips,
                path: path.clone(),
                game: "Captured game".into(),
            })
            .unwrap();
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.index_store.pending() > 0 {
            assert!(Instant::now() < deadline);
            app.poll_index_store(&egui::Context::default());
            std::thread::sleep(Duration::from_millis(1));
        }
        let live = app.library.as_ref().unwrap();
        assert!(live.entries[0].favorite);
        assert_eq!(live.entries[0].collections, ["Best"]);
        assert!(app.index_store.edits.is_empty());
        app.index_store.finish();
        let CommandOutput::LibraryScan(stored) =
            execute(Command::LoadLibrary { index_path: index }).unwrap()
        else {
            panic!("load");
        };
        assert!(stored.entries[0].favorite);
        assert_eq!(stored.entries[0].collections, ["Best"]);
        assert_eq!(stored.entries[0].probe, Some(probe));
        assert_eq!(
            stored.entries[0].recorded_game.as_deref(),
            Some("Captured game")
        );
        assert_eq!(std::fs::read(path).unwrap(), b"unchanged source");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn normal_close_waits_for_accepted_writes_then_closes_after_acknowledgement() {
        let (release, wait) = mpsc::sync_channel(1);
        let worker = Worker::start_with(egui::Context::default(), move |_| {
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(None)
        })
        .unwrap();
        let mut app = crate::ClipCraftApp {
            index_store: State {
                worker: Some(worker),
                ..Default::default()
            },
            ..Default::default()
        };
        app.index_store
            .submit(Write::Edit {
                index: "unused".into(),
                directory: "/clips".into(),
                edit: MetaEdit::CreateCollection { name: "A".into() },
            })
            .unwrap();
        let context = crate::kit::test_context();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let output = context.run(input, |ctx| app.index_save_status(ctx));
        assert!(app.index_store.closing);
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::CancelClose)
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.index_store.pending() > 0 {
            assert!(Instant::now() < deadline);
            app.poll_index_store(&context);
            std::thread::sleep(Duration::from_millis(1));
        }
        let output = context.run(egui::RawInput::default(), |ctx| app.index_save_status(ctx));
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::Close)
        );
        app.index_store.finish();
    }
}

#[cfg(test)]
mod terminal_tests {
    use super::*;
    #[test]
    fn failed_join_is_reported_and_explicit_discard_can_close_a_poisoned_queue() {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let thread = std::thread::spawn(|| panic!("injected terminal worker failure"));
        let mut state = State {
            worker: Some(Worker {
                shared,
                thread: Some(thread),
            }),
            ..Default::default()
        };
        state.finish();
        assert!(state.error.is_some());
        let worker = Worker::start(egui::Context::default()).unwrap();
        let shared = worker.shared.clone();
        let _ = std::thread::spawn(move || {
            let _guard = shared.0.lock().unwrap();
            panic!("injected poison");
        })
        .join();
        let mut state = State {
            worker: Some(worker),
            closing: true,
            ..Default::default()
        };
        state.poll();
        assert!(state.error.is_some());
        state.discard();
        assert_eq!(state.pending(), 0);
        assert!(state.closing);
    }
}
