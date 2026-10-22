//! One serial worker for internal editor recovery metadata.
//!
//! Saves never run on the UI thread. Discard is a barrier after any in-flight save, so a late
//! writer cannot resurrect a draft the person discarded. Normal shutdown can wait at most two
//! seconds for a final checkpoint; a stalled filesystem leaves the earlier atomic checkpoint.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use clipcraft_engine::{Command, CommandOutput, EditorDraft, execute};
use eframe::egui;

pub const EXIT_WAIT: Duration = Duration::from_secs(2);
const STOPPED: &str = "Editor recovery stopped unexpectedly. The last saved draft is unchanged.";
const EXIT_TIMEOUT: &str = "The final editor checkpoint did not finish in time. The previous saved draft is still available.";

#[derive(Debug)]
pub enum RecoveryResult {
    Loaded(Option<Box<EditorDraft>>),
    Saved,
    Discarded,
    Failed(String),
}

enum Operation {
    Load,
    Save(Box<EditorDraft>),
    Discard,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Load,
    Save,
    Discard,
}

impl Operation {
    fn kind(&self) -> Kind {
        match self {
            Self::Load => Kind::Load,
            Self::Save(_) => Kind::Save,
            Self::Discard => Kind::Discard,
        }
    }
}

struct Request {
    operation: Operation,
    reply: SyncSender<RecoveryResult>,
    wake: egui::Context,
}

struct Flight {
    kind: Kind,
    reply: Receiver<RecoveryResult>,
}

/// One operation is active; an explicit discard may wait behind it. Saves are accepted only
/// while idle, leaving debouncing and revision tracking to the editor.
pub struct Recovery {
    sender: Option<SyncSender<Request>>,
    current: Option<Flight>,
    pending_discard: Option<egui::Context>,
    pending_error: Option<String>,
    // Dropping the sender shuts the worker down after its current atomic operation. Do not join
    // a stalled filesystem on the UI thread; completion/disconnection is reported by the reply.
    _worker: Option<JoinHandle<()>>,
}

fn execute_operation(path: &Path, operation: Operation) -> RecoveryResult {
    let command = match operation {
        Operation::Load => Command::LoadEditorDraft {
            path: path.to_owned(),
        },
        Operation::Save(draft) => Command::SaveEditorDraft {
            path: path.to_owned(),
            draft,
        },
        Operation::Discard => Command::DiscardEditorDraft {
            path: path.to_owned(),
        },
    };
    match execute(command) {
        Ok(CommandOutput::EditorDraft(draft)) => RecoveryResult::Loaded(draft),
        Ok(CommandOutput::EditorDraftSaved) => RecoveryResult::Saved,
        Ok(CommandOutput::EditorDraftDiscarded) => RecoveryResult::Discarded,
        Ok(_) => RecoveryResult::Failed(STOPPED.into()),
        Err(error) => RecoveryResult::Failed(error.to_string()),
    }
}

impl Recovery {
    /// Begins loading the internal checkpoint. An unavailable worker is reported by `poll`.
    pub fn new(path: PathBuf, context: &egui::Context) -> Self {
        Self::start(path, context, execute_operation)
    }

    fn start(
        path: PathBuf,
        context: &egui::Context,
        mut execute: impl FnMut(&Path, Operation) -> RecoveryResult + Send + 'static,
    ) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Request>(1);
        let worker = std::thread::Builder::new()
            .name("editor-recovery".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let result =
                        catch_unwind(AssertUnwindSafe(|| execute(&path, request.operation)))
                            .unwrap_or_else(|_| RecoveryResult::Failed(STOPPED.into()));
                    let _ = request.reply.send(result);
                    request.wake.request_repaint();
                }
            });
        let mut recovery = match worker {
            Ok(worker) => Self {
                sender: Some(sender),
                current: None,
                pending_discard: None,
                pending_error: None,
                _worker: Some(worker),
            },
            Err(_) => Self {
                sender: None,
                current: None,
                pending_discard: None,
                pending_error: Some(
                    "Editor recovery could not start. Your clips are unchanged.".into(),
                ),
                _worker: None,
            },
        };
        if recovery.sender.is_some() {
            recovery.submit(Operation::Load, context);
        }
        recovery
    }

    pub fn busy(&self) -> bool {
        self.current.is_some() || self.pending_discard.is_some()
    }

    /// Returns one completed operation, starting a queued discard before another save may begin.
    pub fn poll(&mut self) -> Option<RecoveryResult> {
        if let Some(error) = self.pending_error.take() {
            return Some(RecoveryResult::Failed(error));
        }
        let result = match self.current.as_ref()?.reply.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.current = None;
                self.pending_discard = None;
                self.sender = None;
                return Some(RecoveryResult::Failed(STOPPED.into()));
            }
        };
        self.current = None;
        self.start_discard();
        Some(result)
    }

    /// Accepts a checkpoint only while idle. The editor keeps its revision dirty on `false`.
    pub fn save(&mut self, draft: EditorDraft, context: &egui::Context) -> bool {
        if self.busy() {
            return false;
        }
        self.submit(Operation::Save(Box::new(draft)), context)
    }

    /// Discard is ordered after the current operation and repeated requests coalesce.
    pub fn discard(&mut self, context: &egui::Context) -> bool {
        if self.pending_discard.is_some()
            || self
                .current
                .as_ref()
                .is_some_and(|flight| flight.kind == Kind::Discard)
        {
            return true;
        }
        if self.current.is_some() {
            self.pending_discard = Some(context.clone());
            return true;
        }
        self.submit(Operation::Discard, context)
    }

    /// Final shutdown checkpoint, serialized behind all earlier saves/discards. Pass `Some`
    /// only for a deliberately newer draft; `None` only drains already accepted operations.
    /// Results consumed here are for shutdown and are not subsequently replayed by `poll`.
    /// The deadline also covers earlier operations drained by `finish_pending`.
    pub fn flush(
        &mut self,
        draft: Option<EditorDraft>,
        context: &egui::Context,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut first_error = None;
        for result in self.finish_pending(deadline)? {
            if let RecoveryResult::Failed(error) = result {
                first_error.get_or_insert(error);
            }
        }
        if let Some(draft) = draft {
            if Instant::now() >= deadline {
                return Err(EXIT_TIMEOUT.into());
            }
            if !self.submit(Operation::Save(Box::new(draft)), context) {
                return Err(self.pending_error.take().unwrap_or_else(|| STOPPED.into()));
            }
            for result in self.finish_pending(deadline)? {
                if let RecoveryResult::Failed(error) = result {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Completes accepted operations within an existing shutdown budget. Applying their results
    /// before choosing the final snapshot keeps a pending load/discard from hiding newer edits.
    pub fn finish_pending(&mut self, deadline: Instant) -> Result<Vec<RecoveryResult>, String> {
        // One current operation, one coalesced discard and one dispatch error are the upper bound.
        let mut results = Vec::with_capacity(3);
        if let Some(error) = self.pending_error.take() {
            results.push(RecoveryResult::Failed(error));
        }
        self.start_discard();
        while let Some(flight) = self.current.as_ref() {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(EXIT_TIMEOUT)?;
            match flight.reply.recv_timeout(remaining) {
                Ok(result) => {
                    results.push(result);
                    self.current = None;
                    self.start_discard();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => return Err(EXIT_TIMEOUT.into()),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.current = None;
                    self.pending_discard = None;
                    self.sender = None;
                    return Err(STOPPED.into());
                }
            }
        }
        if let Some(error) = self.pending_error.take() {
            results.push(RecoveryResult::Failed(error));
        }
        Ok(results)
    }

    fn start_discard(&mut self) {
        if self.current.is_none()
            && let Some(context) = self.pending_discard.take()
        {
            self.submit(Operation::Discard, &context);
        }
    }

    fn submit(&mut self, operation: Operation, context: &egui::Context) -> bool {
        let Some(sender) = &self.sender else {
            self.pending_error = Some(STOPPED.into());
            context.request_repaint();
            return false;
        };
        let kind = operation.kind();
        let (reply, receiver) = mpsc::sync_channel(1);
        match sender.try_send(Request {
            operation,
            reply,
            wake: context.clone(),
        }) {
            Ok(()) => {
                self.current = Some(Flight {
                    kind,
                    reply: receiver,
                });
                true
            }
            Err(TrySendError::Full(_)) => {
                self.pending_error =
                    Some("Editor recovery is still busy. The previous draft is unchanged.".into());
                context.request_repaint();
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                self.sender = None;
                self.pending_error = Some(STOPPED.into());
                context.request_repaint();
                false
            }
        }
    }
}

impl Drop for Recovery {
    fn drop(&mut self) {
        self.pending_discard = None;
        self.sender = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn draft(title: &str) -> EditorDraft {
        EditorDraft::new(
            &Default::default(),
            Vec::new(),
            title.into(),
            clipcraft_time::Tick::ZERO,
            None,
        )
        .unwrap()
    }

    fn next(recovery: &mut Recovery) -> RecoveryResult {
        let start = Instant::now();
        loop {
            if let Some(result) = recovery.poll() {
                return result;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "worker did not reply"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn discard_is_a_barrier_after_save_and_repeated_requests_do_not_grow_the_queue() {
        let context = egui::Context::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let (started, starts) = mpsc::channel();
        let (release, releases) = mpsc::channel();
        let mut recovery =
            Recovery::start("editor-draft.json".into(), &context, move |_, operation| {
                match operation {
                    Operation::Load => {
                        log.lock().unwrap().push("load");
                        RecoveryResult::Loaded(None)
                    }
                    Operation::Save(_) => {
                        log.lock().unwrap().push("save");
                        started.send(()).unwrap();
                        releases.recv_timeout(Duration::from_secs(5)).unwrap();
                        RecoveryResult::Saved
                    }
                    Operation::Discard => {
                        log.lock().unwrap().push("discard");
                        RecoveryResult::Discarded
                    }
                }
            });
        assert!(!recovery.save(draft("too early"), &context));
        assert!(matches!(next(&mut recovery), RecoveryResult::Loaded(None)));
        assert!(recovery.save(draft("first"), &context));
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..100 {
            assert!(recovery.discard(&context));
            assert!(!recovery.save(draft("must not overtake"), &context));
        }
        assert_eq!(*seen.lock().unwrap(), ["load", "save"]);
        release.send(()).unwrap();
        assert!(matches!(next(&mut recovery), RecoveryResult::Saved));
        assert!(recovery.busy());
        assert!(matches!(next(&mut recovery), RecoveryResult::Discarded));
        assert!(!recovery.busy());
        assert_eq!(*seen.lock().unwrap(), ["load", "save", "discard"]);
    }

    #[test]
    fn flush_orders_final_checkpoint_after_queued_discard() {
        let context = egui::Context::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let mut recovery =
            Recovery::start("editor-draft.json".into(), &context, move |_, operation| {
                match operation {
                    Operation::Load => RecoveryResult::Loaded(None),
                    Operation::Save(draft) => {
                        log.lock().unwrap().push(draft.title);
                        RecoveryResult::Saved
                    }
                    Operation::Discard => {
                        log.lock().unwrap().push("discard".into());
                        RecoveryResult::Discarded
                    }
                }
            });
        assert!(matches!(next(&mut recovery), RecoveryResult::Loaded(None)));
        assert!(recovery.save(draft("old"), &context));
        assert!(recovery.discard(&context));
        recovery
            .flush(Some(draft("new")), &context, Instant::now() + EXIT_WAIT)
            .unwrap();
        assert_eq!(*seen.lock().unwrap(), ["old", "discard", "new"]);
        assert!(!recovery.busy());
    }

    #[test]
    fn operation_failure_clears_busy_and_does_not_disable_discard() {
        let context = egui::Context::default();
        let mut recovery =
            Recovery::start(
                "editor-draft.json".into(),
                &context,
                |_, operation| match operation {
                    Operation::Load => {
                        RecoveryResult::Failed("The stored draft is damaged.".into())
                    }
                    Operation::Save(_) => RecoveryResult::Failed("The disk is full.".into()),
                    Operation::Discard => RecoveryResult::Discarded,
                },
            );
        assert!(matches!(next(&mut recovery), RecoveryResult::Failed(_)));
        assert!(!recovery.busy());
        assert!(recovery.save(draft("edit"), &context));
        assert!(matches!(next(&mut recovery), RecoveryResult::Failed(_)));
        assert!(!recovery.busy());
        assert!(recovery.discard(&context));
        assert!(matches!(next(&mut recovery), RecoveryResult::Discarded));
    }

    #[test]
    fn shutdown_timeout_is_bounded_and_does_not_queue_a_later_writer() {
        let context = egui::Context::default();
        let (release, releases) = mpsc::channel();
        let (started, starts) = mpsc::channel();
        let mut recovery =
            Recovery::start("editor-draft.json".into(), &context, move |_, operation| {
                match operation {
                    Operation::Load => RecoveryResult::Loaded(None),
                    Operation::Save(_) => {
                        started.send(()).unwrap();
                        releases.recv_timeout(Duration::from_secs(5)).unwrap();
                        RecoveryResult::Saved
                    }
                    Operation::Discard => RecoveryResult::Discarded,
                }
            });
        next(&mut recovery);
        assert!(recovery.save(draft("old"), &context));
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        let started = Instant::now();
        assert!(
            recovery
                .flush(
                    Some(draft("new")),
                    &context,
                    Instant::now() + Duration::from_millis(20)
                )
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(recovery.busy());
        release.send(()).unwrap();
        assert!(matches!(next(&mut recovery), RecoveryResult::Saved));
        assert!(!recovery.busy());
        assert!(
            starts.try_recv().is_err(),
            "the timed-out final save must not start later"
        );
    }

    #[test]
    fn dropping_closes_the_queue_after_the_active_operation() {
        struct Notice(mpsc::Sender<()>);
        impl Drop for Notice {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let context = egui::Context::default();
        let (finished, finishes) = mpsc::channel();
        let notice = Notice(finished);
        let (started, starts) = mpsc::channel();
        let (release, releases) = mpsc::channel();
        let recovery =
            Recovery::start("editor-draft.json".into(), &context, move |_, operation| {
                let _keep_until_worker_exits = &notice;
                assert!(matches!(operation, Operation::Load));
                started.send(()).unwrap();
                releases.recv_timeout(Duration::from_secs(5)).unwrap();
                RecoveryResult::Loaded(None)
            });
        starts.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = Instant::now();
        drop(recovery);
        assert!(before.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        finishes.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn draining_does_not_renew_the_deadline_for_a_final_save() {
        let context = egui::Context::default();
        let (saved, saves) = mpsc::channel();
        let mut recovery =
            Recovery::start("editor-draft.json".into(), &context, move |_, operation| {
                match operation {
                    Operation::Load => RecoveryResult::Loaded(None),
                    Operation::Save(_) => {
                        saved.send(()).unwrap();
                        RecoveryResult::Saved
                    }
                    Operation::Discard => RecoveryResult::Discarded,
                }
            });
        assert!(matches!(next(&mut recovery), RecoveryResult::Loaded(None)));
        let deadline = Instant::now() + Duration::from_millis(20);
        assert!(recovery.finish_pending(deadline).unwrap().is_empty());
        std::thread::sleep(Duration::from_millis(25));
        assert!(
            recovery
                .flush(Some(draft("too late")), &context, deadline)
                .is_err()
        );
        assert!(!recovery.busy());
        assert!(saves.try_recv().is_err());
    }
}
