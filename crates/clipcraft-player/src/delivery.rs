//! Fixed playback-result slots. A slow consumer never queues historical pictures.
use super::{ClipInfoSummary, Command, DecodedFrame, PlayerError, PlayerEvent};
use std::sync::{Mutex, mpsc::Sender};

pub(super) struct Request {
    pub command: Command,
    pub revision: u64,
}
#[derive(Default)]
struct Pending {
    opened: Option<ClipInfoSummary>,
    frame: Option<DecodedFrame>,
    terminal: Option<PlayerEvent>,
    revision: u64,
    closed: bool,
}
#[derive(Default)]
pub(super) struct Delivery(Mutex<Pending>);

pub(super) enum Publication {
    Accepted,
    Superseded,
    Closed,
}
impl Delivery {
    /// Serialize invalidation with command enqueueing, including concurrent callers. The channel
    /// is nonblocking/unbounded; capacity/ordering of control commands is a separate contract.
    pub fn dispatch(&self, command: Command, commands: &Sender<Request>) {
        let Ok(mut pending) = self.0.lock() else {
            let _ = commands.send(Request {
                command: Command::Shutdown,
                revision: 0,
            });
            return;
        };
        if pending.closed {
            return;
        }
        if matches!(
            command,
            Command::Seek(_) | Command::Step(_) | Command::Play | Command::PlayFromCurrent
        ) {
            let Some(revision) = pending.revision.checked_add(1) else {
                pending.frame = None;
                pending.terminal = Some(PlayerEvent::Failed(PlayerError::Crashed));
                pending.closed = true;
                let _ = commands.send(Request {
                    command: Command::Shutdown,
                    revision: pending.revision,
                });
                return;
            };
            pending.revision = revision;
            pending.frame = None;
            if matches!(pending.terminal, Some(PlayerEvent::Ended)) {
                pending.terminal = None;
            }
        }
        if matches!(command, Command::Shutdown) {
            pending.closed = true;
            pending.frame = None;
            pending.opened = None;
            pending.terminal = None;
        }
        if commands
            .send(Request {
                command,
                revision: pending.revision,
            })
            .is_err()
        {
            pending.terminal = Some(PlayerEvent::Failed(PlayerError::Crashed));
            pending.closed = true;
        }
    }
    pub fn publish(&self, event: PlayerEvent, revision: u64) -> Publication {
        let Ok(mut pending) = self.0.lock() else {
            return Publication::Closed;
        };
        if pending.closed {
            return Publication::Closed;
        }
        match event {
            PlayerEvent::Opened(info) => pending.opened = Some(info),
            PlayerEvent::Failed(error) => {
                pending.terminal = Some(PlayerEvent::Failed(error));
                pending.closed = true;
            }
            // The mutex makes the revision check and publication atomic with seek requests.
            _ if revision != pending.revision => return Publication::Superseded,
            PlayerEvent::Frame(frame) => pending.frame = Some(frame),
            PlayerEvent::Ended => pending.terminal = Some(PlayerEvent::Ended),
        }
        Publication::Accepted
    }
    /// Fixed delivery order, including when the UI has not polled since opening the clip.
    pub fn poll(&self) -> Vec<PlayerEvent> {
        let Ok(mut pending) = self.0.lock() else {
            return vec![PlayerEvent::Failed(PlayerError::Crashed)];
        };
        let mut events = Vec::with_capacity(3);
        if let Some(info) = pending.opened.take() {
            events.push(PlayerEvent::Opened(info));
        }
        if let Some(frame) = pending.frame.take() {
            events.push(PlayerEvent::Frame(frame));
        }
        if let Some(event) = pending.terminal.take() {
            events.push(event);
        }
        events
    }
    #[cfg(test)]
    pub fn ended(&self) -> bool {
        self.0
            .lock()
            .is_ok_and(|pending| matches!(pending.terminal, Some(PlayerEvent::Ended)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clipcraft_time::Tick;
    use std::sync::{Arc, Barrier, mpsc};
    fn frame(n: i64) -> PlayerEvent {
        PlayerEvent::Frame(DecodedFrame {
            pts: Tick::from_units(n, 30),
            width: 4,
            height: 4,
            rgba: vec![n as u8; 64],
        })
    }
    fn info() -> ClipInfoSummary {
        ClipInfoSummary {
            duration: Tick::from_units(1, 1),
            width: 4,
            height: 4,
            rate_numerator: 30,
            rate_denominator: 1,
            has_audio: false,
            audio_tracks: Vec::new(),
        }
    }
    #[test]
    fn stalled_consumer_retains_one_latest_picture_and_lifecycle_order() {
        let delivery = Delivery::default();
        assert!(matches!(
            delivery.publish(PlayerEvent::Opened(info()), 0),
            Publication::Accepted
        ));
        for n in 0..10000 {
            assert!(matches!(
                delivery.publish(frame(n), 0),
                Publication::Accepted
            ));
        }
        assert!(matches!(
            delivery.publish(PlayerEvent::Ended, 0),
            Publication::Accepted
        ));
        let pending = delivery.0.lock().unwrap();
        assert_eq!(pending.frame.as_ref().unwrap().rgba.len(), 64);
        drop(pending);
        assert_eq!(
            delivery.poll(),
            vec![PlayerEvent::Opened(info()), frame(9999), PlayerEvent::Ended]
        );
        assert!(delivery.poll().is_empty());
    }
    #[test]
    fn latest_seek_step_and_replay_clear_old_pending_pictures_and_end_markers() {
        for command in [
            Command::Seek(Tick::ZERO),
            Command::Step(-1),
            Command::Play,
            Command::PlayFromCurrent,
        ] {
            let delivery = Delivery::default();
            let (tx, rx) = mpsc::channel();
            delivery.publish(PlayerEvent::Opened(info()), 0);
            delivery.publish(frame(29), 0);
            delivery.publish(PlayerEvent::Ended, 0);
            delivery.dispatch(command, &tx);
            let request = rx.recv().unwrap();
            assert_eq!(request.revision, 1);
            assert!(matches!(
                delivery.publish(frame(28), 0),
                Publication::Superseded
            ));
            assert!(matches!(
                delivery.publish(PlayerEvent::Ended, 0),
                Publication::Superseded
            ));
            assert_eq!(delivery.poll(), vec![PlayerEvent::Opened(info())]);
            assert!(matches!(
                delivery.publish(frame(5), 1),
                Publication::Accepted
            ));
            assert_eq!(delivery.poll(), vec![frame(5)]);
        }
    }
    #[test]
    fn a_decode_finishing_after_a_new_seek_cannot_publish_its_old_picture() {
        let delivery = Arc::new(Delivery::default());
        let gate = Arc::new(Barrier::new(2));
        let worker_delivery = delivery.clone();
        let worker_gate = gate.clone();
        let worker = std::thread::spawn(move || {
            worker_gate.wait();
            worker_delivery.publish(frame(1), 0)
        });
        let (tx, rx) = mpsc::channel();
        delivery.dispatch(Command::Seek(Tick::from_units(20, 30)), &tx);
        gate.wait();
        assert!(matches!(worker.join().unwrap(), Publication::Superseded));
        assert!(delivery.poll().is_empty());
        delivery.publish(frame(20), rx.recv().unwrap().revision);
        assert_eq!(delivery.poll(), vec![frame(20)]);
    }
    #[test]
    fn concurrent_controls_enqueue_their_revisions_in_the_same_order() {
        let delivery = Arc::new(Delivery::default());
        let (tx, rx) = mpsc::channel();
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let delivery = delivery.clone();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for _ in 0..512 {
                        delivery.dispatch(Command::Seek(Tick::ZERO), &tx);
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        for revision in 1..=2048 {
            assert_eq!(rx.recv().unwrap().revision, revision);
        }
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn failure_is_reported_after_the_latest_frame_and_cannot_be_overwritten() {
        let delivery = Delivery::default();
        let (tx, _rx) = mpsc::channel();
        delivery.publish(PlayerEvent::Opened(info()), 0);
        delivery.publish(frame(4), 0);
        delivery.publish(PlayerEvent::Failed(PlayerError::DecodeFailed), 0);
        delivery.dispatch(Command::Play, &tx);
        assert!(matches!(
            delivery.publish(PlayerEvent::Ended, 0),
            Publication::Closed
        ));
        assert_eq!(
            delivery.poll(),
            vec![
                PlayerEvent::Opened(info()),
                frame(4),
                PlayerEvent::Failed(PlayerError::DecodeFailed)
            ]
        );
        assert!(delivery.poll().is_empty());
    }
    #[test]
    fn shutdown_drops_pixels_and_a_disconnected_worker_is_reported() {
        let delivery = Delivery::default();
        let (tx, rx) = mpsc::channel();
        delivery.publish(PlayerEvent::Opened(info()), 0);
        delivery.publish(frame(4), 0);
        delivery.dispatch(Command::Shutdown, &tx);
        assert!(matches!(rx.recv().unwrap().command, Command::Shutdown));
        assert!(delivery.poll().is_empty());
        assert!(matches!(delivery.publish(frame(9), 0), Publication::Closed));
        let delivery = Delivery::default();
        drop(rx);
        delivery.dispatch(Command::Pause, &tx);
        assert_eq!(
            delivery.poll(),
            vec![PlayerEvent::Failed(PlayerError::Crashed)]
        );
    }
    #[test]
    fn revision_exhaustion_and_poisoned_state_fail_closed() {
        let delivery = Delivery::default();
        let (tx, rx) = mpsc::channel();
        delivery.0.lock().unwrap().revision = u64::MAX;
        delivery.publish(frame(9), u64::MAX);
        delivery.dispatch(Command::Seek(Tick::ZERO), &tx);
        assert!(matches!(rx.recv().unwrap().command, Command::Shutdown));
        assert_eq!(
            delivery.poll(),
            vec![PlayerEvent::Failed(PlayerError::Crashed)]
        );
        let delivery = Arc::new(Delivery::default());
        let poison = delivery.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = poison.0.lock().unwrap();
                panic!("synthetic lock poison");
            })
            .join()
            .is_err()
        );
        assert_eq!(
            delivery.poll(),
            vec![PlayerEvent::Failed(PlayerError::Crashed)]
        );
        assert!(matches!(delivery.publish(frame(1), 0), Publication::Closed));
        delivery.dispatch(Command::Seek(Tick::ZERO), &tx);
        assert!(matches!(rx.recv().unwrap().command, Command::Shutdown));
    }
}
