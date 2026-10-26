//! Decoding a few pictures ahead of the clock while a clip plays.
//!
//! One picture in about ten is far more expensive to decode than the rest (a full intra picture
//! can take ten times the time of a predicted one), and a single decoder thread that hits one has
//! to skip pictures to catch up. With a short queue of finished pictures the slow ones are decoded
//! during the time the fast ones saved, so playback keeps every picture as long as the average
//! decode fits in a picture time.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use filmcraft_media::SharedSource;

use crate::{ClipInfo, DecodedFrame, decode_at, scheduling};

/// How many pictures are kept ready.
const DEPTH: usize = 6;
/// Pictures this big (RGBA bytes, all ready ones together) are not worth holding.
const MAX_QUEUE_BYTES: usize = 96 * 1024 * 1024;

#[derive(Default)]
struct State {
    /// Bumped whenever the queue is thrown away, so a decode that finishes late is dropped.
    generation: u64,
    /// The next picture to decode, and the last one worth decoding.
    next: i64,
    last: i64,
    running: bool,
    stop: bool,
    ready: BTreeMap<i64, DecodedFrame>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) struct Ahead {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Ahead {
    pub(crate) fn start(source: SharedSource, info: ClipInfo) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let worker = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("clipcraft-decode-ahead".to_owned())
                .spawn(move || work(&shared, &source, &info))
                .ok()
        };
        Self { shared, worker }
    }

    /// The finished picture `frame`, if it was decoded ahead. Older pictures are dropped.
    pub(crate) fn take(&self, frame: i64) -> Option<DecodedFrame> {
        let mut state = lock(&self.shared);
        state.ready = state.ready.split_off(&frame);
        let found = state.ready.remove(&frame);
        if found.is_some() {
            self.shared.wake.notify_all();
        }
        found
    }

    /// Plays on: keeps `from..=to` decoded as the clock allows.
    pub(crate) fn run(&self, from: i64, to: i64) {
        let mut state = lock(&self.shared);
        if !state.running || from + (DEPTH as i64) < state.next {
            // Starting, or the clip jumped back: nothing decoded so far is the next picture.
            state.generation += 1;
            state.ready.clear();
            state.next = from;
        } else if from > state.next {
            // The clock got ahead of the decoder: do not decode pictures that are already past.
            state.next = from;
        }
        state.last = to;
        state.running = true;
        self.shared.wake.notify_all();
    }

    /// Playback stopped or jumped: throw away everything decoded ahead.
    pub(crate) fn clear(&self) {
        let mut state = lock(&self.shared);
        if state.running || !state.ready.is_empty() {
            state.generation += 1;
            state.ready.clear();
            state.running = false;
        }
        self.shared.wake.notify_all();
    }
}

impl Drop for Ahead {
    fn drop(&mut self) {
        {
            let mut state = lock(&self.shared);
            state.stop = true;
            state.generation += 1;
            state.ready.clear();
        }
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn work(shared: &Shared, source: &SharedSource, info: &ClipInfo) {
    loop {
        // Wait for something to decode.
        let (frame, generation) = {
            let mut state = lock(shared);
            loop {
                if state.stop {
                    return;
                }
                let bytes: usize = state.ready.values().map(|picture| picture.rgba.len()).sum();
                if state.running
                    && state.next <= state.last
                    && state.ready.len() < DEPTH
                    && bytes < MAX_QUEUE_BYTES
                {
                    break;
                }
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            let frame = state.next;
            state.next += 1;
            (frame, state.generation)
        };
        let stale = || {
            let state = lock(shared);
            state.stop || state.generation != generation
        };
        let Ok(_permit) = scheduling::foreground(stale) else {
            continue;
        };
        let at = info.frame_rate.tick_of(frame);
        let decoded = catch_unwind(AssertUnwindSafe(|| decode_at(source, info, at)));
        drop(_permit);
        let Ok(Ok(picture)) = decoded else {
            continue;
        };
        let mut state = lock(shared);
        if state.generation == generation && !state.stop {
            state.ready.insert(frame, picture);
        }
    }
}
