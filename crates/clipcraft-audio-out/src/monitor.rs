//! Local microphone listening test: recording DSP on a worker, bounded PCM to the output callback.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use clipcraft_engine::MicrophoneTestPlan;
use clipcraft_record::{AudioFormat, PcmMonitor};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream, StreamConfig};

static BUSY: AtomicBool = AtomicBool::new(false);

struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Queue {
    frames: Mutex<VecDeque<[f32; 2]>>,
    capacity: AtomicUsize,
}

impl Queue {
    fn clear(&self) {
        self.frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
    fn push(&self, samples: &[f32], channels: usize) -> Result<(), String> {
        let capacity = self.capacity.load(Ordering::Relaxed);
        if capacity == 0 || !(1..=2).contains(&channels) {
            return Err("The microphone playback queue is not configured.".into());
        }
        let mut frames = self
            .frames
            .lock()
            .map_err(|_| "The microphone playback queue failed. Restart the test.".to_owned())?;
        // Keep only the newest 200 ms, including after output stalls. Never replay a stale backlog.
        for frame in samples.chunks_exact(channels) {
            if frames.len() >= capacity {
                frames.pop_front();
            }
            frames.push_back([frame[0], *frame.get(1).unwrap_or(&frame[0])]);
        }
        Ok(())
    }
    fn fill<T: SizedSample + cpal::FromSample<f32>>(
        &self,
        data: &mut [T],
        channels: usize,
        shared: &Shared,
    ) {
        for sample in data.iter_mut() {
            *sample = T::from_sample(0.0);
        }
        if shared.stop.load(Ordering::Relaxed) {
            return;
        }
        let mut frames = match self.frames.try_lock() {
            Ok(frames) => frames,
            Err(TryLockError::WouldBlock) => return,
            Err(TryLockError::Poisoned(_)) => {
                shared.output_failed.store(true, Ordering::Relaxed);
                return;
            }
        };
        let mut delivered = 0;
        for frame in data.chunks_mut(channels) {
            let Some(input) = frames.pop_front() else {
                break;
            };
            for (channel, sample) in frame.iter_mut().enumerate().take(2) {
                *sample = T::from_sample(input[channel]);
            }
            delivered += 1;
        }
        shared.played.fetch_add(delivered, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Shared {
    pending: Mutex<Option<MicrophoneTestPlan>>,
    error: Mutex<Option<String>>,
    opening_since: Mutex<Option<Instant>>,
    stop: AtomicBool,
    ready: AtomicBool,
    finished: AtomicBool,
    output_failed: AtomicBool,
    level: AtomicU32,
    played: AtomicU64,
}

/// A sixty-second, unsaved microphone listening test. Dropping it silences output immediately.
/// Opening and DSP stay off the UI/device callbacks. At most one worker can exist per process,
/// including a cancelled worker still waiting for the OS to finish opening a device.
pub struct MicTest {
    shared: Arc<Shared>,
    queue: Arc<Queue>,
    thread: Option<JoinHandle<()>>,
}

impl MicTest {
    pub fn start(plan: MicrophoneTestPlan) -> Result<Self, String> {
        if plan.device.len() > 1024 || plan.gain_percent > 100 || !(1..=2).contains(&plan.channels)
        {
            return Err("The microphone test settings are invalid.".into());
        }
        Self::start_worker(move |state, samples| run(plan, state, samples))
    }

    fn start_worker(
        task: impl FnOnce(&Arc<Shared>, &Arc<Queue>) -> Result<(), String> + Send + 'static,
    ) -> Result<Self, String> {
        if BUSY
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(
                "The previous microphone test is still stopping. Try again in a moment.".into(),
            );
        }
        let permit = Permit;
        let shared = Arc::new(Shared::default());
        *shared
            .opening_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        let queue = Arc::new(Queue::default());
        let state = shared.clone();
        let samples = queue.clone();
        let thread = std::thread::Builder::new()
            .name("clipcraft-mic-test".into())
            .spawn(move || {
                let _permit = permit;
                let result = catch_unwind(AssertUnwindSafe(|| task(&state, &samples)));
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error),
                    Err(_) => Some("The microphone test worker failed. Restart the test.".into()),
                };
                if let Some(error) = error {
                    *state.error.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(error.chars().take(512).collect());
                }
                state.ready.store(false, Ordering::Relaxed);
                samples.clear();
                state.finished.store(true, Ordering::Release);
            })
            .map_err(|_| "The microphone test worker could not start.".to_owned())?;
        Ok(Self {
            shared,
            queue,
            thread: Some(thread),
        })
    }

    /// Replaces the one pending settings update; the active test follows current settings.
    pub fn update(&self, plan: MicrophoneTestPlan) -> Result<(), String> {
        if plan.device.len() > 1024 || plan.gain_percent > 100 || !(1..=2).contains(&plan.channels)
        {
            return Err("The microphone test settings are invalid.".into());
        }
        *self
            .shared
            .pending
            .lock()
            .map_err(|_| "The microphone test settings failed.".to_owned())? = Some(plan);
        Ok(())
    }
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }
    pub fn ready(&self) -> bool {
        self.shared.ready.load(Ordering::Relaxed)
    }
    pub fn finished(&self) -> bool {
        self.check_open_timeout();
        self.shared.finished.load(Ordering::Acquire)
            || (self.shared.stop.load(Ordering::Relaxed) && self.error().is_some())
    }
    pub fn error(&self) -> Option<String> {
        self.check_open_timeout();
        self.shared
            .error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn check_open_timeout(&self) {
        let timed_out = self
            .shared
            .opening_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|started| started.elapsed() >= Duration::from_secs(15));
        if timed_out {
            self.shared.stop.store(true, Ordering::Relaxed);
            let mut error = self
                .shared
                .error
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if error.is_none() {
                *error = Some("Opening the microphone test timed out. Check the audio devices, close other audio apps and try again.".into());
            }
        }
    }
    /// Frames consumed by the real output callback, excluding underrun silence.
    pub fn played_frames(&self) -> u64 {
        self.shared.played.load(Ordering::Relaxed)
    }
}

impl Drop for MicTest {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.queue.clear();
        if let Some(thread) = self.thread.take().filter(JoinHandle::is_finished)
            && thread.join().is_err()
        {
            *self
                .shared
                .error
                .lock()
                .unwrap_or_else(PoisonError::into_inner) =
                Some("The microphone test worker could not finish.".into());
        }
        // An opening OS call cannot be cancelled safely. Its worker holds the single permit,
        // observes stop before publishing samples, then drops both devices on that same thread.
    }
}

fn processor(
    plan: &MicrophoneTestPlan,
    format: AudioFormat,
) -> Result<(PcmMonitor, clipcraft_record::AudioControl), String> {
    let source = crate::DeviceCapture::open(crate::CaptureKind::Microphone, &plan.device)
        .map_err(|error| error.to_string())?;
    let input = plan.input(Box::new(source))?;
    let control = input.control.clone();
    Ok((PcmMonitor::new(input, format)?, control))
}

fn run(
    mut plan: MicrophoneTestPlan,
    shared: &Arc<Shared>,
    initial_queue: &Arc<Queue>,
) -> Result<(), String> {
    let host = crate::host();
    let device = host
        .default_output_device()
        .ok_or("No playback device was found. Connect headphones and restart the test.")?;
    let supported = device
        .default_output_config()
        .map_err(|_| "The playback format could not be read.".to_owned())?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    if !(8_000..=192_000).contains(&config.sample_rate.0) || !(1..=8).contains(&config.channels) {
        return Err(
            "The microphone test needs an 8–192 kHz playback device with 1–8 channels.".into(),
        );
    }
    // Queue capacity is configured before publishing the callback, with at most 38,400 frames.
    let queue = initial_queue;
    queue
        .capacity
        .store(config.sample_rate.0 as usize / 5, Ordering::Relaxed);
    let mut format = AudioFormat {
        sample_rate: config.sample_rate.0,
        channels: config.channels.min(plan.channels),
    };
    let (mut input, mut control) = processor(&plan, format)?;
    let stream = match sample_format {
        SampleFormat::F32 => output::<f32>(&device, &config, queue, shared),
        SampleFormat::I16 => output::<i16>(&device, &config, queue, shared),
        SampleFormat::U16 => output::<u16>(&device, &config, queue, shared),
        SampleFormat::I32 => output::<i32>(&device, &config, queue, shared),
        _ => {
            return Err("The playback sample format is unsupported for microphone testing.".into());
        }
    }?;
    if shared.stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    stream
        .play()
        .map_err(|_| "The playback device could not start. Check your headphones.".to_owned())?;
    shared.ready.store(true, Ordering::Relaxed);
    *shared
        .opening_since
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = None;
    let started = Instant::now();
    let mut samples = Vec::new();
    while !shared.stop.load(Ordering::Relaxed) && started.elapsed() < Duration::from_secs(60) {
        if shared.output_failed.load(Ordering::Relaxed) {
            return Err(
                "Microphone playback stopped. Check the output device and restart the test.".into(),
            );
        }
        let update = shared
            .pending
            .lock()
            .map_err(|_| "The microphone test settings failed.".to_owned())?
            .take();
        if let Some(next) = update {
            queue.clear();
            if plan.device != next.device
                || plan.noise_suppression != next.noise_suppression
                || plan.mono != next.mono
                || plan.channels != next.channels
            {
                shared.ready.store(false, Ordering::Relaxed);
                *shared
                    .opening_since
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
                // Close the old microphone before opening its replacement; no parallel capture.
                drop(input);
                format.channels = config.channels.min(next.channels);
                (input, control) = processor(&next, format)?;
                shared.ready.store(true, Ordering::Relaxed);
                *shared
                    .opening_since
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = None;
            } else {
                control.set_gain_percent(next.gain_percent);
            }
            plan = next;
        }
        samples.clear();
        input
            .read(&mut samples)
            .map_err(|error| error.to_string())?;
        shared
            .level
            .store(crate::peak(&samples).to_bits(), Ordering::Relaxed);
        queue.push(&samples, usize::from(format.channels))?;
        std::thread::sleep(Duration::from_millis(5));
    }
    queue.clear();
    Ok(())
}

fn output<T: SizedSample + cpal::FromSample<f32> + Send + 'static>(
    device: &cpal::Device,
    config: &StreamConfig,
    queue: &Arc<Queue>,
    shared: &Arc<Shared>,
) -> Result<Stream, String> {
    let queue = queue.clone();
    let state = shared.clone();
    let failed = shared.clone();
    let channels = usize::from(config.channels);
    device.build_output_stream(config, move |data: &mut [T], _| queue.fill(data, channels, &state),
        move |_| failed.output_failed.store(true, Ordering::Relaxed), None)
        .map_err(|_| "The microphone playback device could not open. Check the output device and restart the test.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_worker_failure_finishes_and_pending_updates_stay_bounded() {
        let (send, wait) = std::sync::mpsc::channel();
        let test = MicTest::start_worker(move |_, _| {
            let _ = wait.recv();
            Err("Generated device loss".into())
        })
        .unwrap();
        assert!(
            MicTest::start_worker(|_, _| Ok(())).is_err(),
            "only one worker may own the device"
        );
        let mut plan =
            MicrophoneTestPlan::from_settings(&clipcraft_engine::RecordingSettings::default());
        for gain in 0..=100 {
            plan.gain_percent = gain;
            test.update(plan.clone()).unwrap();
        }
        assert_eq!(
            test.shared
                .pending
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .gain_percent,
            100
        );
        plan.channels = 0;
        assert!(test.update(plan).is_err());
        *test.shared.opening_since.lock().unwrap() =
            Some(Instant::now().checked_sub(Duration::from_secs(16)).unwrap());
        assert!(
            test.finished(),
            "a blocked OS open must leave the UI's running state"
        );
        assert!(test.error().unwrap().contains("timed out"));
        assert!(
            MicTest::start_worker(|_, _| Ok(())).is_err(),
            "timed-out OS work must retain the sole permit until cleanup"
        );
        send.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !test.shared.finished.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(test.finished());
        assert_eq!(test.error().as_deref(), Some("Generated device loss"));
        drop(test);
        while BUSY.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!BUSY.load(Ordering::SeqCst));
    }

    #[test]
    fn playback_queue_preserves_stereo_bounds_backlog_and_silences_after_stop() {
        let queue = Queue {
            frames: Mutex::new(VecDeque::new()),
            capacity: AtomicUsize::new(2),
        };
        let shared = Shared::default();
        queue.push(&[0.1, 0.2, 0.3, 0.4, 0.5, 0.6], 2).unwrap();
        let mut out = [9.0_f32; 12];
        queue.fill(&mut out, 4, &shared);
        assert_eq!(
            out,
            [0.3, 0.4, 0.0, 0.0, 0.5, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(shared.played.load(Ordering::Relaxed), 2);
        queue.push(&[0.8, 0.4], 1).unwrap();
        shared.stop.store(true, Ordering::Relaxed);
        queue.fill(&mut out, 2, &shared);
        assert_eq!(out, [0.0; 12]);
    }

    #[test]
    fn playback_callback_never_waits_for_the_worker() {
        let queue = Queue {
            frames: Mutex::new(VecDeque::new()),
            capacity: AtomicUsize::new(2),
        };
        let shared = Shared::default();
        let _lock = queue.frames.lock().unwrap();
        let mut out = [1.0_f32; 4];
        queue.fill(&mut out, 2, &shared);
        assert_eq!(out, [0.0; 4]);
        assert!(!shared.output_failed.load(Ordering::Relaxed));
    }

    #[test]
    #[ignore = "explicit native microphone-to-headphones test; plays locally, saves no audio"]
    fn real_processed_microphone_test_plays_and_applies_live_controls() {
        let mut plan = MicrophoneTestPlan::from_settings(&clipcraft_engine::RecordingSettings {
            microphone_noise_suppression: true,
            ..Default::default()
        });
        let test = MicTest::start(plan.clone()).unwrap();
        let started = Instant::now();
        while test.played_frames() < 48_000 && started.elapsed() < Duration::from_secs(15) {
            assert_eq!(test.error(), None);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            test.played_frames() > 0,
            "real output callback received no microphone frames"
        );
        plan.gain_percent = 0;
        test.update(plan.clone()).unwrap();
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(test.level(), 0.0);
        plan.gain_percent = 50;
        plan.mono = true;
        test.update(plan).unwrap();
        let before = test.played_frames();
        std::thread::sleep(Duration::from_secs(1));
        assert_eq!(test.error(), None);
        assert!(test.played_frames() > before);
        println!(
            "Native processed microphone playback: {} output frames; live zero gain and mono update passed; no audio saved.",
            test.played_frames()
        );
        drop(test);
        let deadline = Instant::now() + Duration::from_secs(3);
        while BUSY.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !BUSY.load(Ordering::SeqCst),
            "microphone test worker did not stop"
        );
    }
}
