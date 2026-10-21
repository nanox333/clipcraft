//! Bounded Media Foundation event delivery. EndGetEvent runs inside Invoke as documented.
#![allow(unsafe_code)]
#![warn(clippy::undocumented_unsafe_blocks)]

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::time::Duration;
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Media::MediaFoundation::*;
use windows::core::{AgileReference, IUnknown, IUnknownImpl, implement};

const MAX_EVENTS: usize = 64;
struct State<T = u32> {
    events: VecDeque<T>,
    error: Option<String>,
    closed: bool,
}
impl<T> Default for State<T> {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            error: None,
            closed: false,
        }
    }
}
impl<T> State<T> {
    fn receive(&mut self, event: Result<T, String>) -> bool {
        if self.closed || self.error.is_some() {
            return false;
        }
        match event {
            Ok(event) if self.events.len() < MAX_EVENTS => self.events.push_back(event),
            Ok(_) => self.error = Some("encoder event queue exceeded its bound".to_owned()),
            Err(error) => self.error = Some(error),
        }
        self.error.is_none()
    }
    fn take(&mut self) -> Result<Option<T>, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(self.events.pop_front())
    }
    fn close(&mut self) {
        self.closed = true;
        self.events.clear();
    }
}
#[derive(Default)]
struct Signal {
    state: Mutex<State>,
    wake: Condvar,
}
struct Owner {
    generator: AgileReference<IMFMediaEventGenerator>,
}

#[implement(IMFAsyncCallback)]
struct Callback {
    owner: Weak<Owner>,
    signal: Arc<Signal>,
}

impl IMFAsyncCallback_Impl for Callback_Impl {
    fn GetParameters(&self, flags: *mut u32, queue: *mut u32) -> windows::core::Result<()> {
        if flags.is_null() || queue.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: non-null writable out-parameters provided by Media Foundation. A single
        // outstanding request serializes this callback on the documented asynchronous queue.
        unsafe {
            *flags = 0;
            *queue = MFASYNC_CALLBACK_QUEUE_MULTITHREADED;
        }
        Ok(())
    }
    fn Invoke(&self, result: windows::core::Ref<'_, IMFAsyncResult>) -> windows::core::Result<()> {
        let result = result
            .as_ref()
            .ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
        let Some(owner) = self.owner.upgrade() else {
            return Ok(());
        };
        let generator = match owner.generator.resolve() {
            Ok(generator) => generator,
            Err(error) => {
                self.signal
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .receive(Err(format!("resolving encoder events failed: {error}")));
                self.signal.wake.notify_one();
                return Ok(());
            }
        };
        // SAFETY: the agile reference resolves on this callback thread. Finish exactly one
        // request on the same generator and read the owned event here; no COM event crosses threads.
        let event = unsafe {
            generator.EndGetEvent(result).and_then(|event| {
                event.GetStatus()?.ok()?;
                event.GetType()
            })
        };
        let mut state = self
            .signal
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return Ok(());
        }
        let rearm = state
            .receive(event.map_err(|error| format!("receiving encoder event failed: {error}")));
        drop(state);
        self.signal.wake.notify_one();
        if rearm {
            let callback: IMFAsyncCallback = self.to_interface();
            // SAFETY: EndGetEvent finished this request. Re-arm once, holding owned COM refs;
            // weak owner prevents a generator/callback reference cycle during shutdown.
            let requested = unsafe { generator.BeginGetEvent(&callback, None::<&IUnknown>) };
            if let Err(error) = requested {
                let mut state = self
                    .signal
                    .state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if !state.closed {
                    state.error = Some(format!("requesting encoder event failed: {error}"));
                }
                drop(state);
                self.signal.wake.notify_one();
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Events {
    _owner: Arc<Owner>,
    _callback: IMFAsyncCallback,
    signal: Arc<Signal>,
}

impl Events {
    pub(crate) fn new(generator: IMFMediaEventGenerator) -> Result<Self, String> {
        let owner = Arc::new(Owner {
            generator: AgileReference::new(&generator)
                .map_err(|e| format!("making encoder event reference agile failed: {e}"))?,
        });
        let signal = Arc::new(Signal::default());
        let callback: IMFAsyncCallback = Callback {
            owner: Arc::downgrade(&owner),
            signal: Arc::clone(&signal),
        }
        .into();
        // SAFETY: valid generator and owned callback; no other request has been started.
        unsafe { generator.BeginGetEvent(&callback, None::<&IUnknown>) }
            .map_err(|e| format!("requesting encoder events failed: {e}"))?;
        Ok(Self {
            _owner: owner,
            _callback: callback,
            signal,
        })
    }
    pub(crate) fn try_next(&self) -> Result<Option<u32>, String> {
        let _time = crate::d3d11::measure_event();
        let mut state = self
            .signal
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.take()
    }
    pub(crate) fn wait(&self, timeout: Duration) {
        let _time = crate::d3d11::measure_wait();
        let state = self
            .signal
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.events.is_empty() && state.error.is_none() && !state.closed {
            let _wait = self
                .signal
                .wake
                .wait_timeout_while(state, timeout, |s| {
                    s.events.is_empty() && s.error.is_none() && !s.closed
                })
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
    pub(crate) fn close(&self) {
        let mut state = self
            .signal
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.close();
        drop(state);
        self.signal.wake.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_state_can_cross_threads_without_raw_com_references() {
        fn send_and_sync<T: Send + Sync>() {}
        send_and_sync::<Owner>();
        send_and_sync::<Signal>();
        send_and_sync::<Callback>();
    }

    #[test]
    fn events_are_ordered_bounded_and_overflow_stops_rearming() {
        let mut state = State::default();
        for index in 0..MAX_EVENTS {
            assert!(state.receive(Ok(index)));
        }
        assert!(!state.receive(Ok(MAX_EVENTS)));
        assert_eq!(state.events.len(), MAX_EVENTS);
        assert!(state.take().unwrap_err().contains("bound"));
        assert!(!state.receive(Ok(99)));
        assert_eq!(state.events.len(), MAX_EVENTS);
        let mut ordered = State::default();
        assert!(ordered.receive(Ok(1)));
        assert!(ordered.receive(Ok(2)));
        assert_eq!(ordered.take().unwrap(), Some(1));
        assert_eq!(ordered.take().unwrap(), Some(2));
        assert_eq!(ordered.take().unwrap(), None);
    }

    #[test]
    fn close_releases_queued_ownership_and_rejects_late_events() {
        let value = Arc::new(());
        let mut state = State::default();
        assert!(state.receive(Ok(Arc::clone(&value))));
        assert_eq!(Arc::strong_count(&value), 2);
        state.close();
        assert_eq!(Arc::strong_count(&value), 1);
        assert!(!state.receive(Ok(Arc::clone(&value))));
        assert_eq!(Arc::strong_count(&value), 1);
        assert!(state.take().unwrap().is_none());
    }

    #[test]
    fn asynchronous_failure_is_sticky_and_visible() {
        let mut state = State::<usize>::default();
        assert!(!state.receive(Err("device gone".to_owned())));
        assert_eq!(state.take().unwrap_err(), "device gone");
        assert!(!state.receive(Ok(1)));
        assert_eq!(state.take().unwrap_err(), "device gone");
    }
}
