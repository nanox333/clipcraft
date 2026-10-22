//! Visible timeline pictures and audio envelopes, decoded by one cancellable worker.
//!
//! The UI owns the request queue and LRU. It never decodes media, and uploads at most two small
//! pictures per frame. Closing the editor cancels work without replacing a still-running worker.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};

use clipcraft_engine::{
    Command, CommandOutput, TimelineAnalysis, TimelineRequest, WaveformBucket, execute_cancellable,
};
use clipcraft_time::Tick;
use eframe::egui;

const MAX_PENDING: usize = 8;
const MAX_ENTRIES: usize = 256;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const UPLOADS_PER_FRAME: usize = 2;
const STAMP_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RGBA_BYTES: usize = 12 * 160 * 90 * 4;

type AnalysisResult = Result<TimelineAnalysis, String>;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Key {
    path: PathBuf,
    start: Tick,
    end: Tick,
    frames: u16,
    buckets: u16,
}
impl Key {
    fn request(&self) -> TimelineRequest {
        TimelineRequest {
            path: self.path.clone(),
            start: self.start,
            end: self.end,
            frames: self.frames,
            buckets: self.buckets,
        }
    }
    fn valid(&self) -> bool {
        !self.path.as_os_str().is_empty()
            && self.path.as_os_str().len() <= 4096
            && self.start >= Tick::ZERO
            && self.end > self.start
            && self.end.0.checked_sub(self.start.0).is_some_and(|ticks| {
                (clipcraft_time::TICKS_PER_SECOND / 30..=clipcraft_time::TICKS_PER_SECOND * 1800)
                    .contains(&ticks)
            })
            && (1..=12).contains(&self.frames)
            && (16..=2048).contains(&self.buckets)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Fingerprint {
    bytes: u64,
    modified: Option<SystemTime>,
}
fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    Some(Fingerprint {
        bytes: metadata.len(),
        modified: metadata.modified().ok(),
    })
}
struct Stamp {
    path: PathBuf,
    value: Option<Fingerprint>,
    checked: Instant,
}
struct Pending {
    key: Key,
    stamp: Option<Fingerprint>,
    seen: u64,
}
struct Task {
    key: Key,
    stamp: Option<Fingerprint>,
    generation: u64,
    cancel: Arc<AtomicBool>,
    wake: egui::Context,
}
struct Reply {
    key: Key,
    stamp: Option<Fingerprint>,
    generation: u64,
    result: AnalysisResult,
    changed: bool,
}
struct Running {
    key: Key,
    stamp: Option<Fingerprint>,
    cancel: Arc<AtomicBool>,
    seen: u64,
}

/// A sampled source picture, at its original source timestamp.
pub struct CachedFrame {
    pub position: Tick,
    pub texture: egui::TextureHandle,
}
/// Pictures may arrive over several frames; the full envelope is available first.
pub struct CachedTimeline {
    pub frames: Vec<CachedFrame>,
    pub waveform: Vec<WaveformBucket>,
    pub has_audio: bool,
}
struct Entry {
    key: Key,
    stamp: Option<Fingerprint>,
    value: CachedTimeline,
    uploading: VecDeque<clipcraft_engine::TimelineFrame>,
    error: Option<String>,
    bytes: usize,
}

struct Worker {
    sender: mpsc::SyncSender<Task>,
    receiver: mpsc::Receiver<Reply>,
    stopped: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}
impl Worker {
    fn start() -> Result<Self, ()> {
        let (sender, requests) = mpsc::sync_channel::<Task>(1);
        let (finished, receiver) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let thread = std::thread::Builder::new()
            .name("editor-timeline".into())
            .spawn(move || {
                while !worker_stopped.load(Ordering::Relaxed) {
                    let task = match requests.recv_timeout(Duration::from_millis(100)) {
                        Ok(task) => task,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    };
                    let before = fingerprint(&task.key.path);
                    let result = if before.is_none() {
                        Err("The timeline source is unavailable or is not a regular file.".into())
                    } else if before != task.stamp || task.cancel.load(Ordering::Relaxed) {
                        Err("Timeline source changed or its analysis was cancelled.".to_owned())
                    } else {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            execute_cancellable(
                                Command::AnalyzeEditorTimeline {
                                    request: Box::new(task.key.request()),
                                },
                                &task.cancel,
                            )
                            .and_then(|output| match output {
                                CommandOutput::EditorTimeline(analysis) => Ok(*analysis),
                                _ => Err(clipcraft_engine::EngineError::MediaUnsupported),
                            })
                            .map_err(|error| error.to_string())
                        }))
                        .unwrap_or_else(|_| Err("Timeline analysis stopped unexpectedly.".into()))
                    };
                    let changed = before != task.stamp || fingerprint(&task.key.path) != before;
                    if worker_stopped.load(Ordering::Relaxed) {
                        return;
                    }
                    let reply = Reply {
                        key: task.key,
                        stamp: task.stamp,
                        generation: task.generation,
                        result,
                        changed,
                    };
                    // The UI schedules another task only after receiving this result, so this
                    // bounded channel has room. Dropping the UI also disconnects the receiver.
                    if finished.send(reply).is_err() {
                        return;
                    }
                    task.wake.request_repaint();
                }
            })
            .map_err(|_| ())?;
        Ok(Self {
            sender,
            receiver,
            stopped,
            thread,
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

/// An editor-session cache; call `poll` once before drawing and `request` for visible cuts.
#[derive(Default)]
pub struct TimelineCache {
    entries: VecDeque<Entry>,
    stamps: VecDeque<Stamp>,
    pending: VecDeque<Pending>,
    running: Option<Running>,
    worker: Option<Worker>,
    start_failed: bool,
    generation: u64,
    epoch: u64,
    bytes: usize,
    texture_number: u64,
}
impl TimelineCache {
    pub fn request(
        &mut self,
        context: &egui::Context,
        path: &Path,
        start: Tick,
        end: Tick,
        frames: u16,
        buckets: u16,
    ) {
        let key = Key {
            path: path.to_path_buf(),
            start,
            end,
            frames,
            buckets,
        };
        if !key.valid() {
            return;
        }
        let stamp = self.source_stamp(path);
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.key == key && entry.stamp == stamp)
        {
            if let Some(entry) = self.entries.remove(index) {
                self.entries.push_back(entry);
            }
            return;
        }
        if let Some(running) = self
            .running
            .as_mut()
            .filter(|running| running.key == key && running.stamp == stamp)
        {
            running.seen = self.epoch;
            return;
        }
        if let Some(pending) = self
            .pending
            .iter_mut()
            .find(|pending| pending.key == key && pending.stamp == stamp)
        {
            pending.seen = self.epoch;
            return;
        }
        if self.pending.len() < MAX_PENDING {
            self.pending.push_back(Pending {
                key,
                stamp,
                seen: self.epoch,
            });
        }
        self.schedule(context);
    }

    pub fn get(
        &self,
        path: &Path,
        start: Tick,
        end: Tick,
        frames: u16,
        buckets: u16,
    ) -> Option<&CachedTimeline> {
        self.find(path, start, end, frames, buckets)
            .filter(|entry| entry.error.is_none())
            .map(|entry| &entry.value)
    }
    /// A failed analysis is remembered until the source changes or the cache is cleared.
    pub fn error(
        &self,
        path: &Path,
        start: Tick,
        end: Tick,
        frames: u16,
        buckets: u16,
    ) -> Option<&str> {
        self.find(path, start, end, frames, buckets)?
            .error
            .as_deref()
    }
    fn find(
        &self,
        path: &Path,
        start: Tick,
        end: Tick,
        frames: u16,
        buckets: u16,
    ) -> Option<&Entry> {
        self.entries.iter().find(|entry| {
            entry.key.path == path
                && entry.key.start == start
                && entry.key.end == end
                && entry.key.frames == frames
                && entry.key.buckets == buckets
        })
    }

    pub fn poll(&mut self, context: &egui::Context) {
        self.epoch = self.epoch.wrapping_add(1);
        self.pending
            .retain(|pending| self.epoch.wrapping_sub(pending.seen) <= 1);
        if let Some(running) = &self.running
            && self.epoch.wrapping_sub(running.seen) > 1
        {
            running.cancel.store(true, Ordering::Relaxed);
        }
        let reply = self
            .worker
            .as_ref()
            .and_then(|worker| match worker.receiver.try_recv() {
                Ok(reply) => Some(Ok(reply)),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(())),
            });
        if let Some(reply) = reply {
            match reply {
                Ok(reply) => self.receive(reply),
                Err(()) => {
                    if let Some(running) = self.running.take()
                        && !running.cancel.load(Ordering::Relaxed)
                    {
                        self.failed(
                            running.key,
                            running.stamp,
                            "Timeline analysis stopped unexpectedly.".into(),
                        );
                    }
                    // A disconnected worker is already finished or finishing. It cannot produce
                    // more work; do not replace it until its thread has actually exited.
                }
            }
        }
        self.upload(context);
        self.schedule(context);
        if self.running.is_some()
            || !self.pending.is_empty()
            || self.entries.iter().any(|entry| !entry.uploading.is_empty())
        {
            context.request_repaint_after(Duration::from_millis(25));
        }
    }

    /// Release all pictures and queued work. Keep one cancelled worker until it acknowledges
    /// cancellation, so repeated editor navigation cannot create an unbounded set of decoders.
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(running) = &self.running {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.entries.clear();
        self.pending.clear();
        self.stamps.clear();
        self.bytes = 0;
        self.start_failed = false;
    }

    fn source_stamp(&mut self, path: &Path) -> Option<Fingerprint> {
        let now = Instant::now();
        if let Some(index) = self.stamps.iter().position(|stamp| stamp.path == path) {
            if now.duration_since(self.stamps[index].checked) < STAMP_INTERVAL {
                return self.stamps[index].value;
            }
            let value = fingerprint(path);
            let previous = self.stamps[index].value;
            if let Some(stamp) = self.stamps.get_mut(index) {
                stamp.value = value;
                stamp.checked = now;
            }
            if value != previous {
                self.invalidate(path);
            }
            return value;
        }
        let value = fingerprint(path);
        // The stamp list can evict a path before the texture LRU does. A new stamp must
        // therefore invalidate old pictures as well, even when no previous stamp remains.
        if self
            .entries
            .iter()
            .any(|entry| entry.key.path == path && entry.stamp != value)
            || self
                .running
                .as_ref()
                .is_some_and(|running| running.key.path == path && running.stamp != value)
        {
            self.invalidate(path);
        }
        if self.stamps.len() == MAX_ENTRIES {
            self.stamps.pop_front();
        }
        self.stamps.push_back(Stamp {
            path: path.to_path_buf(),
            value,
            checked: now,
        });
        value
    }
    fn invalidate(&mut self, path: &Path) {
        self.entries.retain(|entry| entry.key.path != path);
        self.bytes = self.entries.iter().map(|entry| entry.bytes).sum();
        self.pending.retain(|pending| pending.key.path != path);
        if let Some(running) = self
            .running
            .as_ref()
            .filter(|running| running.key.path == path)
        {
            running.cancel.store(true, Ordering::Relaxed);
        }
    }
    fn schedule(&mut self, context: &egui::Context) {
        if self.running.is_some() {
            return;
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread.is_finished())
        {
            self.worker = None;
        }
        let Some(pending) = self.pending.pop_front() else {
            return;
        };
        if self.worker.is_none() && !self.start_failed {
            match Worker::start() {
                Ok(worker) => self.worker = Some(worker),
                Err(()) => self.start_failed = true,
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let task = Task {
            key: pending.key.clone(),
            stamp: pending.stamp,
            generation: self.generation,
            cancel: Arc::clone(&cancel),
            wake: context.clone(),
        };
        let sent = self
            .worker
            .as_ref()
            .ok_or(())
            .and_then(|worker| worker.sender.try_send(task).map_err(|_| ()));
        if sent.is_ok() {
            self.running = Some(Running {
                key: pending.key,
                stamp: pending.stamp,
                cancel,
                seen: pending.seen,
            });
        } else {
            self.failed(
                pending.key,
                pending.stamp,
                "Timeline analysis could not start. Reopen the editor to try again.".into(),
            );
        }
    }
    fn receive(&mut self, reply: Reply) {
        let Some(running) = self.running.take() else {
            return;
        };
        if running.key != reply.key
            || running.stamp != reply.stamp
            || reply.generation != self.generation
            || running.cancel.load(Ordering::Relaxed)
        {
            return;
        }
        if reply.changed {
            self.invalidate(&reply.key.path);
            // Keep the old stamp until its normal refresh. Otherwise an actively written
            // source would launch and discard another decode on every UI frame.
            self.failed(
                reply.key,
                reply.stamp,
                "The source changed while preparing its timeline. Waiting for the file to settle."
                    .into(),
            );
            return;
        }
        match reply.result {
            Ok(analysis) => {
                let Some(bytes) = checked_bytes(&reply.key, &analysis) else {
                    self.failed(
                        reply.key,
                        reply.stamp,
                        "Timeline data could not be read safely.".into(),
                    );
                    return;
                };
                self.make_room(bytes);
                self.entries.push_back(Entry {
                    key: reply.key,
                    stamp: reply.stamp,
                    value: CachedTimeline {
                        frames: Vec::new(),
                        waveform: analysis.waveform,
                        has_audio: analysis.has_audio,
                    },
                    uploading: analysis.frames.into(),
                    error: None,
                    bytes,
                });
                self.bytes += bytes;
            }
            Err(error) => self.failed(reply.key, reply.stamp, error),
        }
    }
    fn failed(&mut self, key: Key, stamp: Option<Fingerprint>, message: String) {
        self.make_room(0);
        self.entries.push_back(Entry {
            key,
            stamp,
            value: CachedTimeline {
                frames: Vec::new(),
                waveform: Vec::new(),
                has_audio: false,
            },
            uploading: VecDeque::new(),
            error: Some(message.chars().take(512).collect()),
            bytes: 0,
        });
    }
    fn make_room(&mut self, incoming: usize) {
        while self.entries.len() >= MAX_ENTRIES || self.bytes.saturating_add(incoming) > MAX_BYTES {
            let Some(old) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
    }
    fn upload(&mut self, context: &egui::Context) {
        let mut remaining = UPLOADS_PER_FRAME;
        // Requests touch their entries at the back. Prioritize newly visible pictures
        // over uploads left over from a previous viewport.
        for entry in self.entries.iter_mut().rev() {
            while remaining > 0 {
                let Some(frame) = entry.uploading.pop_front() else {
                    break;
                };
                let dimensions = [frame.width as usize, frame.height as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(dimensions, &frame.rgba);
                self.texture_number = self.texture_number.wrapping_add(1);
                let texture = context.load_texture(
                    format!("editor-timeline-{}", self.texture_number),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                entry.value.frames.push(CachedFrame {
                    position: frame.position,
                    texture,
                });
                remaining -= 1;
            }
            if remaining == 0 {
                break;
            }
        }
    }
}
impl Drop for TimelineCache {
    fn drop(&mut self) {
        if let Some(running) = &self.running {
            running.cancel.store(true, Ordering::Relaxed);
        }
    }
}

fn checked_bytes(key: &Key, analysis: &TimelineAnalysis) -> Option<usize> {
    if analysis.frames.is_empty()
        || analysis.frames.len() > usize::from(key.frames)
        || analysis.waveform.len() > usize::from(key.buckets)
    {
        return None;
    }
    let mut bytes = analysis
        .waveform
        .len()
        .checked_mul(std::mem::size_of::<WaveformBucket>())?;
    let mut rgba_bytes = 0_usize;
    let mut previous = None;
    for frame in &analysis.frames {
        if !(1..=160).contains(&frame.width)
            || !(1..=90).contains(&frame.height)
            || frame.position < key.start
            || frame.position >= key.end
            || previous.is_some_and(|position| frame.position <= position)
        {
            return None;
        }
        let expected = (frame.width as usize)
            .checked_mul(frame.height as usize)?
            .checked_mul(4)?;
        if expected != frame.rgba.len() {
            return None;
        }
        rgba_bytes = rgba_bytes.checked_add(expected)?;
        previous = Some(frame.position);
    }
    if rgba_bytes > MAX_RGBA_BYTES {
        return None;
    }
    for bucket in &analysis.waveform {
        if bucket.min == i16::MIN || bucket.max == i16::MIN || bucket.min > bucket.max {
            return None;
        }
    }
    bytes = bytes.checked_add(rgba_bytes)?;
    (bytes <= MAX_BYTES).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Key {
        Key {
            path: "generated.mp4".into(),
            start: Tick::ZERO,
            end: Tick::from_units(1, 1),
            frames: 3,
            buckets: 16,
        }
    }
    fn analysis() -> TimelineAnalysis {
        TimelineAnalysis {
            frames: (0..3)
                .map(|n| clipcraft_engine::TimelineFrame {
                    position: Tick::from_units(n, 3),
                    width: 2,
                    height: 2,
                    rgba: vec![120; 16],
                })
                .collect(),
            waveform: vec![
                WaveformBucket {
                    min: -16_384,
                    max: 24_576
                };
                16
            ],
            has_audio: true,
        }
    }
    fn running(key: &Key) -> Running {
        Running {
            key: key.clone(),
            stamp: None,
            cancel: Arc::new(AtomicBool::new(false)),
            seen: 0,
        }
    }
    fn deliver(cache: &mut TimelineCache, data: TimelineAnalysis) {
        let key = key();
        cache.running = Some(running(&key));
        cache.receive(Reply {
            key,
            stamp: None,
            generation: cache.generation,
            result: Ok(data),
            changed: false,
        });
    }
    #[test]
    fn pictures_upload_a_few_per_frame_and_clear_releases_them() {
        let context = egui::Context::default();
        let mut cache = TimelineCache::default();
        deliver(&mut cache, analysis());
        assert!(cache.entries[0].value.frames.is_empty());
        cache.upload(&context);
        assert_eq!(cache.entries[0].value.frames.len(), 2);
        cache.upload(&context);
        assert_eq!(cache.entries[0].value.frames.len(), 3);
        let allocated = cache.bytes;
        assert!(allocated > 0);
        cache.clear();
        assert!(cache.entries.is_empty());
        assert_eq!(cache.bytes, 0);
    }
    #[test]
    fn stale_and_cancelled_results_never_replace_visible_data() {
        let mut cache = TimelineCache::default();
        let key = key();
        cache.running = Some(running(&key));
        let generation = cache.generation;
        cache.clear();
        assert!(
            cache
                .running
                .as_ref()
                .unwrap()
                .cancel
                .load(Ordering::Relaxed)
        );
        cache.receive(Reply {
            key: key.clone(),
            stamp: None,
            generation,
            result: Ok(analysis()),
            changed: false,
        });
        assert!(cache.entries.is_empty());
        cache.running = Some(running(&key));
        cache.receive(Reply {
            key: key.clone(),
            stamp: None,
            generation: cache.generation,
            result: Ok(analysis()),
            changed: true,
        });
        assert!(
            cache
                .get(&key.path, key.start, key.end, key.frames, key.buckets)
                .is_none()
        );
        assert!(
            cache
                .error(&key.path, key.start, key.end, key.frames, key.buckets)
                .is_some()
        );
    }
    #[test]
    fn changing_sources_wait_for_the_next_stamp_check_before_retrying() {
        let mut cache = TimelineCache::default();
        let key = key();
        cache.stamps.push_back(Stamp {
            path: key.path.clone(),
            value: None,
            checked: Instant::now(),
        });
        cache.running = Some(running(&key));
        cache.receive(Reply {
            key: key.clone(),
            stamp: None,
            generation: cache.generation,
            result: Ok(analysis()),
            changed: true,
        });
        for _ in 0..3 {
            cache.request(
                &egui::Context::default(),
                &key.path,
                key.start,
                key.end,
                key.frames,
                key.buckets,
            );
        }
        assert!(cache.pending.is_empty());
        assert!(cache.running.is_none());
        assert_eq!(cache.entries.len(), 1);
    }
    #[test]
    fn a_disconnected_cancelled_worker_cannot_restore_a_stale_failure() {
        let mut cache = TimelineCache::default();
        let key = key();
        cache.running = Some(running(&key));
        let (sender, requests) = mpsc::sync_channel(1);
        let (finished, receiver) = mpsc::sync_channel(1);
        drop(requests);
        drop(finished);
        cache.worker = Some(Worker {
            sender,
            receiver,
            stopped: Arc::new(AtomicBool::new(false)),
            thread: std::thread::spawn(|| {}),
        });
        cache.clear();
        cache.poll(&egui::Context::default());
        assert!(cache.running.is_none());
        assert!(
            cache
                .error(&key.path, key.start, key.end, key.frames, key.buckets)
                .is_none()
        );
        assert!(cache.entries.is_empty());
    }
    #[test]
    fn cache_limits_failed_entries_and_rejects_bad_pixels_or_envelopes() {
        let mut cache = TimelineCache::default();
        for n in 0..MAX_ENTRIES + 30 {
            let mut key = key();
            key.path = format!("{n}.mp4").into();
            cache.failed(key, None, "Unsupported".into());
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        let mut invalid = analysis();
        invalid.frames[0].rgba.pop();
        assert!(checked_bytes(&key(), &invalid).is_none());
        let mut invalid = analysis();
        invalid.waveform[0].min = i16::MIN;
        assert!(checked_bytes(&key(), &invalid).is_none());
        let mut invalid = analysis();
        invalid.frames[1].position = Tick::ZERO;
        assert!(checked_bytes(&key(), &invalid).is_none());
        let mut invalid = key();
        invalid.start = Tick(i64::MIN);
        assert!(!invalid.valid());
    }
    #[test]
    fn decoded_storage_evicts_oldest_entries_before_reaching_its_byte_limit() {
        let mut cache = TimelineCache::default();
        for n in 0..50 {
            let mut key = key();
            key.path = format!("storage-{n}.mp4").into();
            key.frames = 12;
            let mut data = analysis();
            data.frames = (0..12)
                .map(|index| clipcraft_engine::TimelineFrame {
                    position: Tick::from_units(index, 12),
                    width: 160,
                    height: 90,
                    rgba: vec![0; 160 * 90 * 4],
                })
                .collect();
            cache.running = Some(running(&key));
            cache.receive(Reply {
                key,
                stamp: None,
                generation: cache.generation,
                result: Ok(data),
                changed: false,
            });
            assert!(cache.bytes <= MAX_BYTES);
        }
        assert!(cache.entries.len() < 50);
        assert!(
            cache
                .entries
                .iter()
                .all(|entry| entry.key.path != Path::new("storage-0.mp4"))
        );
    }
    #[test]
    fn a_missing_source_stamp_cannot_keep_pictures_from_an_older_file() {
        let mut cache = TimelineCache::default();
        let key = key();
        let actual = fingerprint(&key.path);
        let older = Some(Fingerprint {
            bytes: actual.map_or(1, |value| value.bytes.wrapping_add(1)),
            modified: None,
        });
        cache.failed(key.clone(), older, "Old source".into());
        assert!(cache.stamps.is_empty());
        assert_eq!(cache.source_stamp(&key.path), actual);
        assert!(cache.entries.is_empty());
    }
    #[test]
    fn queue_discards_work_not_requested_on_the_new_viewport() {
        let mut cache = TimelineCache::default();
        let context = egui::Context::default();
        // Keep a fake in-flight job so polling does not start a real decoder.
        cache.running = Some(running(&key()));
        for n in 0..MAX_PENDING + 3 {
            let path = PathBuf::from(format!("pending-{n}.mp4"));
            cache.request(&context, &path, Tick::ZERO, Tick::from_units(1, 1), 3, 16);
        }
        assert_eq!(cache.pending.len(), MAX_PENDING);
        cache.request(
            &context,
            Path::new("pending-0.mp4"),
            Tick::ZERO,
            Tick::from_units(1, 1),
            3,
            16,
        );
        assert_eq!(cache.pending.len(), MAX_PENDING);
        cache.poll(&context);
        assert_eq!(cache.pending.len(), MAX_PENDING);
        cache.poll(&context);
        assert!(cache.pending.is_empty());
        assert!(
            cache
                .running
                .as_ref()
                .unwrap()
                .cancel
                .load(Ordering::Relaxed)
        );
    }
}
