//! Cooperative admission for decoder stages, not CPU-core or codec-thread reservation.
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

const SLOTS: usize = 3;
const BACKGROUND_SLOTS: usize = 1;
const MAX_WAITERS: usize = 64;
const MAX_BACKGROUND_WAITERS: usize = 48;
const CANCEL_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Eq, PartialEq)]
enum Priority {
    Foreground,
    Background,
}
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Error {
    Cancelled,
    Unavailable,
}
#[derive(Default)]
struct State {
    active: usize,
    background: usize,
    waiting: usize,
    foreground_waiting: usize,
}
#[derive(Default)]
struct Coordinator {
    state: Mutex<State>,
    wake: Condvar,
}

pub(super) struct Permit<'a> {
    coordinator: &'a Coordinator,
    priority: Priority,
}
struct Waiting<'a> {
    coordinator: &'a Coordinator,
    priority: Priority,
    registered: bool,
}
impl State {
    fn remove_waiter(&mut self, priority: Priority) {
        self.waiting = self.waiting.saturating_sub(1);
        if priority == Priority::Foreground {
            self.foreground_waiting = self.foreground_waiting.saturating_sub(1);
        }
    }
}
impl Coordinator {
    fn acquire(
        &self,
        priority: Priority,
        cancelled: impl Fn() -> bool,
    ) -> Result<Permit<'_>, Error> {
        // Cancellation callbacks may lock the player/audio state, so never run them under ours.
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let mut waiting = {
            let mut state = self.state.lock().map_err(|_| Error::Unavailable)?;
            if state.waiting >= MAX_WAITERS
                || (priority == Priority::Background
                    && state.waiting.saturating_sub(state.foreground_waiting)
                        >= MAX_BACKGROUND_WAITERS)
            {
                return Err(Error::Unavailable);
            }
            state.waiting += 1;
            if priority == Priority::Foreground {
                state.foreground_waiting += 1;
            }
            Waiting {
                coordinator: self,
                priority,
                registered: true,
            }
        };
        loop {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let mut state = self.state.lock().map_err(|_| Error::Unavailable)?;
            if state.active < SLOTS
                && (priority == Priority::Foreground
                    || (state.background < BACKGROUND_SLOTS && state.foreground_waiting == 0))
            {
                state.remove_waiter(priority);
                waiting.registered = false;
                state.active += 1;
                if priority == Priority::Background {
                    state.background += 1;
                }
                return Ok(Permit {
                    coordinator: self,
                    priority,
                });
            }
            let (state, _) = self
                .wake
                .wait_timeout(state, CANCEL_POLL)
                .map_err(|_| Error::Unavailable)?;
            drop(state);
        }
    }
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if self.registered {
            if let Ok(mut state) = self.coordinator.state.lock() {
                state.remove_waiter(self.priority);
            }
            self.coordinator.wake.notify_all();
        }
    }
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.coordinator.state.lock() {
            state.active = state.active.saturating_sub(1);
            if self.priority == Priority::Background {
                state.background = state.background.saturating_sub(1);
            }
        }
        self.coordinator.wake.notify_all();
    }
}
fn shared() -> &'static Coordinator {
    static SHARED: OnceLock<Coordinator> = OnceLock::new();
    SHARED.get_or_init(Coordinator::default)
}
pub(super) fn foreground(cancelled: impl Fn() -> bool) -> Result<Permit<'static>, Error> {
    shared().acquire(Priority::Foreground, cancelled)
}
pub(super) fn background(cancel: &std::sync::atomic::AtomicBool) -> Result<Permit<'static>, Error> {
    shared().acquire(Priority::Background, || {
        cancel.load(std::sync::atomic::Ordering::Relaxed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Instant;
    fn wait_for(coordinator: &Coordinator, predicate: impl Fn(&State) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if predicate(&coordinator.state.lock().unwrap()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "coordinator did not reach expected state"
            );
            std::thread::yield_now();
        }
    }
    #[test]
    fn background_leaves_two_foreground_slots_and_only_one_background_stage_runs() {
        let coordinator = Arc::new(Coordinator::default());
        let background = coordinator.acquire(Priority::Background, || false).unwrap();
        let one = coordinator.acquire(Priority::Foreground, || false).unwrap();
        let two = coordinator.acquire(Priority::Foreground, || false).unwrap();
        let (ready, rx) = mpsc::channel();
        let c = coordinator.clone();
        let worker = std::thread::spawn(move || {
            let _permit = c.acquire(Priority::Background, || false).unwrap();
            ready.send(()).unwrap();
        });
        wait_for(&coordinator, |s| s.waiting == 1);
        drop(one);
        drop(two);
        assert!(
            rx.try_recv().is_err(),
            "second background stage bypassed the first"
        );
        drop(background);
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.join().unwrap();
        assert_eq!(coordinator.state.lock().unwrap().active, 0);
    }
    #[test]
    fn a_queued_foreground_request_wins_the_next_available_slot() {
        let coordinator = Arc::new(Coordinator::default());
        let mut held: Vec<_> = (0..SLOTS)
            .map(|_| coordinator.acquire(Priority::Foreground, || false).unwrap())
            .collect();
        let (order, rx) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let c = coordinator.clone();
        let tx = order.clone();
        let bg = std::thread::spawn(move || {
            let _permit = c.acquire(Priority::Background, || false).unwrap();
            tx.send("background").unwrap();
        });
        wait_for(&coordinator, |s| s.waiting == 1);
        let c = coordinator.clone();
        let fg = std::thread::spawn(move || {
            let _permit = c.acquire(Priority::Foreground, || false).unwrap();
            order.send("foreground").unwrap();
            released.recv().unwrap();
        });
        wait_for(&coordinator, |s| {
            s.waiting == 2 && s.foreground_waiting == 1
        });
        drop(held.pop());
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            "foreground"
        );
        assert!(rx.try_recv().is_err());
        release.send(()).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            "background"
        );
        fg.join().unwrap();
        bg.join().unwrap();
        drop(held);
        let state = coordinator.state.lock().unwrap();
        assert_eq!(
            (state.active, state.waiting, state.foreground_waiting),
            (0, 0, 0)
        );
    }
    #[test]
    fn cancelled_waiters_unregister_without_needing_a_slot_to_be_released() {
        for priority in [Priority::Foreground, Priority::Background] {
            let coordinator = Arc::new(Coordinator::default());
            let held: Vec<_> = (0..SLOTS)
                .map(|_| coordinator.acquire(Priority::Foreground, || false).unwrap())
                .collect();
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let c = coordinator.clone();
            let worker = std::thread::spawn(move || {
                c.acquire(priority, || flag.load(Ordering::SeqCst)).err()
            });
            wait_for(&coordinator, |s| s.waiting == 1);
            cancel.store(true, Ordering::SeqCst);
            assert_eq!(worker.join().unwrap(), Some(Error::Cancelled));
            let state = coordinator.state.lock().unwrap();
            assert_eq!(
                (state.active, state.waiting, state.foreground_waiting),
                (SLOTS, 0, 0)
            );
            drop(state);
            drop(held);
            assert!(coordinator.acquire(Priority::Background, || false).is_ok());
        }
    }
    #[test]
    fn permits_release_on_panic_and_cancel_callbacks_do_not_run_under_the_lock() {
        let coordinator = Coordinator::default();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _permit = coordinator.acquire(Priority::Background, || false).unwrap();
                panic!("synthetic decode failure");
            }))
            .is_err()
        );
        assert_eq!(coordinator.state.lock().unwrap().active, 0);
        let _permit = coordinator
            .acquire(Priority::Foreground, || {
                assert!(coordinator.state.try_lock().is_ok());
                false
            })
            .unwrap();
    }
    #[test]
    fn exhausted_waiting_capacity_and_poisoned_state_fail_explicitly() {
        let coordinator = Arc::new(Coordinator::default());
        coordinator.state.lock().unwrap().waiting = MAX_BACKGROUND_WAITERS;
        assert!(matches!(
            coordinator.acquire(Priority::Background, || false),
            Err(Error::Unavailable)
        ));
        // A background burst cannot consume all foreground waiting registrations.
        let foreground = coordinator.acquire(Priority::Foreground, || false).unwrap();
        drop(foreground);
        coordinator.state.lock().unwrap().waiting = MAX_WAITERS;
        assert!(matches!(
            coordinator.acquire(Priority::Foreground, || false),
            Err(Error::Unavailable)
        ));
        assert!(matches!(
            coordinator.acquire(Priority::Background, || true),
            Err(Error::Cancelled)
        ));
        coordinator.state.lock().unwrap().waiting = 0;
        let c = coordinator.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = c.state.lock().unwrap();
                panic!("synthetic coordinator poison");
            })
            .join()
            .is_err()
        );
        assert!(matches!(
            coordinator.acquire(Priority::Foreground, || false),
            Err(Error::Unavailable)
        ));
    }
    #[test]
    fn concurrent_stages_remain_within_active_and_background_budgets() {
        let coordinator = Arc::new(Coordinator::default());
        let start = Arc::new(Barrier::new(8));
        let active = Arc::new(AtomicUsize::new(0));
        let background = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for n in 0..8 {
            let c = coordinator.clone();
            let gate = start.clone();
            let active = active.clone();
            let background = background.clone();
            workers.push(std::thread::spawn(move || {
                gate.wait();
                for _ in 0..100 {
                    let priority = if n % 2 == 0 {
                        Priority::Foreground
                    } else {
                        Priority::Background
                    };
                    let permit = c.acquire(priority, || false).unwrap();
                    assert!(active.fetch_add(1, Ordering::SeqCst) < SLOTS);
                    if priority == Priority::Background {
                        assert_eq!(background.fetch_add(1, Ordering::SeqCst), 0);
                    }
                    std::thread::yield_now();
                    if priority == Priority::Background {
                        background.fetch_sub(1, Ordering::SeqCst);
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let state = coordinator.state.lock().unwrap();
        assert_eq!(
            (
                state.active,
                state.background,
                state.waiting,
                state.foreground_waiting
            ),
            (0, 0, 0, 0)
        );
    }
}
