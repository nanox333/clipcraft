//! Recording sound from a microphone, or from what the computer is playing, as a
//! [`clipcraft_record::AudioSource`].
//!
//! The operating system's stream object cannot always move between threads, so each capture runs
//! on a thread of its own that owns the stream; the device callback only appends to a bounded
//! queue, and the recorder drains it.

use clipcraft_record::{AudioError, AudioFormat, AudioSource};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream, StreamConfig};
use std::collections::VecDeque;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// Seconds of sound the queue holds before the oldest is dropped.
const QUEUE_SECONDS: usize = 2;

/// What to record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureKind {
    /// A microphone or other input.
    Microphone,
    /// Everything an output device is playing (the game and the system sounds).
    SystemOutput,
}

/// Why a device could not be recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureError {
    /// There is no such device.
    NoDevice,
    /// The device offers no format ClipCraft can read.
    UnsupportedFormat,
    /// The device refused to open or start. Check that ClipCraft may use it.
    StartFailed,
    /// This computer cannot record what it is playing, and why.
    SystemAudioUnavailable(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDevice => formatter.write_str("No sound device was found."),
            Self::UnsupportedFormat => {
                formatter.write_str("The sound device uses a format ClipCraft cannot read.")
            }
            Self::StartFailed => formatter.write_str(
                "The sound device could not be started. Check that ClipCraft may use it.",
            ),
            Self::SystemAudioUnavailable(why) => {
                write!(formatter, "Game and system sound cannot be recorded: {why}")
            }
        }
    }
}

impl std::error::Error for CaptureError {}

/// Samples waiting to be read, bounded, oldest dropped first.
#[derive(Debug)]
pub struct SampleQueue {
    samples: Mutex<VecDeque<f32>>,
    capacity: usize,
    failed: AtomicBool,
}

impl SampleQueue {
    /// A queue that holds at most `capacity` samples.
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
            failed: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<f32>> {
        self.samples.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Appends samples, dropping the oldest ones that no longer fit.
    pub fn push(&self, incoming: impl ExactSizeIterator<Item = f32>) {
        let mut samples = self.lock();
        let incoming_len = incoming.len();
        if incoming_len >= self.capacity {
            samples.clear();
            samples.extend(incoming.skip(incoming_len - self.capacity));
            return;
        }
        let overflow = (samples.len() + incoming_len).saturating_sub(self.capacity);
        let drop_count = overflow.min(samples.len());
        samples.drain(..drop_count);
        samples.extend(incoming);
    }

    /// Moves everything waiting into `out`.
    pub fn drain_into(&self, out: &mut Vec<f32>) {
        out.extend(self.lock().drain(..));
    }

    /// Marks the device as gone.
    pub fn fail(&self) {
        self.failed.store(true, Ordering::SeqCst);
    }

    /// True once the device reported an error.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    /// Samples waiting.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True when nothing is waiting.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A device being recorded. Dropping it closes the device.
pub struct DeviceCapture {
    kind: CaptureKind,
    requested: String,
    label: String,
    format: AudioFormat,
    queue: Arc<SampleQueue>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct Opened {
    label: String,
    format: AudioFormat,
}

impl DeviceCapture {
    /// Opens the named device (the default when `name` is empty) and starts recording it.
    pub fn open(kind: CaptureKind, name: &str) -> Result<Self, CaptureError> {
        let stop = Arc::new(AtomicBool::new(false));
        let (reply, receive) = mpsc::channel();
        let requested = name.to_owned();
        let thread_stop = stop.clone();
        let thread_name = requested.clone();
        let (queue_send, queue_receive) = mpsc::channel::<Arc<SampleQueue>>();
        let thread = std::thread::Builder::new()
            .name("clipcraft-sound-in".to_owned())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    run(kind, &thread_name, &queue_send, &thread_stop, &reply)
                }));
                if outcome.is_err() {
                    let _ = reply.send(Err(CaptureError::StartFailed));
                }
            })
            .map_err(|_| CaptureError::StartFailed)?;
        let outcome = receive
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or(Err(CaptureError::StartFailed));
        match outcome {
            Ok(opened) => {
                let queue = queue_receive
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|_| CaptureError::StartFailed)?;
                Ok(Self {
                    kind,
                    requested,
                    label: opened.label,
                    format: opened.format,
                    queue,
                    stop,
                    thread: Some(thread),
                })
            }
            Err(error) => {
                stop.store(true, Ordering::SeqCst);
                let _ = thread.join();
                Err(error)
            }
        }
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for DeviceCapture {
    fn drop(&mut self) {
        self.close();
    }
}

impl AudioSource for DeviceCapture {
    fn name(&self) -> String {
        self.label.clone()
    }

    fn format(&self) -> AudioFormat {
        self.format
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        self.queue.drain_into(out);
        if self.queue.failed() {
            return Err(AudioError::DeviceLost(self.label.clone()));
        }
        Ok(())
    }

    fn recover(&mut self) -> bool {
        match Self::open(self.kind, &self.requested) {
            Ok(fresh) => {
                // The new device may have another format; only take it when it matches, since the
                // recorder set up its conversion for the old one.
                if fresh.format == self.format {
                    self.close();
                    *self = fresh;
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        }
    }
}

/// Finds the device to record.
fn find_device(kind: CaptureKind, name: &str) -> Result<cpal::Device, CaptureError> {
    let host = cpal::default_host();
    let by_name = |mut devices: Box<dyn Iterator<Item = cpal::Device>>| -> Option<cpal::Device> {
        devices.find(|device| name.is_empty() || device.name().is_ok_and(|n| n == name))
    };
    match kind {
        CaptureKind::Microphone => {
            let named = (!name.is_empty())
                .then(|| host.input_devices().ok().and_then(|d| by_name(Box::new(d))))
                .flatten();
            named
                .or_else(|| host.default_input_device())
                .ok_or(CaptureError::NoDevice)
        }
        CaptureKind::SystemOutput => system_device(&host, name, by_name),
    }
}

#[cfg(windows)]
fn system_device(
    host: &cpal::Host,
    name: &str,
    by_name: impl Fn(Box<dyn Iterator<Item = cpal::Device>>) -> Option<cpal::Device>,
) -> Result<cpal::Device, CaptureError> {
    // Windows lets an output device be opened as an input: that records what it plays.
    let named = (!name.is_empty())
        .then(|| {
            host.output_devices()
                .ok()
                .and_then(|d| by_name(Box::new(d)))
        })
        .flatten();
    named
        .or_else(|| host.default_output_device())
        .ok_or(CaptureError::NoDevice)
}

#[cfg(not(windows))]
fn system_device(
    host: &cpal::Host,
    _name: &str,
    _by_name: impl Fn(Box<dyn Iterator<Item = cpal::Device>>) -> Option<cpal::Device>,
) -> Result<cpal::Device, CaptureError> {
    // Without loopback, use an input the sound server offers that carries the output: PipeWire
    // and PulseAudio publish "Monitor of ..." inputs.
    host.input_devices()
        .ok()
        .and_then(|mut devices| {
            devices.find(|device| {
                device
                    .name()
                    .is_ok_and(|n| n.to_lowercase().contains("monitor"))
            })
        })
        .ok_or_else(|| {
            CaptureError::SystemAudioUnavailable(
                "this computer offers no \"monitor\" input for what it is playing".to_owned(),
            )
        })
}

fn run(
    kind: CaptureKind,
    name: &str,
    queue_out: &mpsc::Sender<Arc<SampleQueue>>,
    stop: &AtomicBool,
    reply: &mpsc::Sender<Result<Opened, CaptureError>>,
) {
    let opened = (|| {
        let device = find_device(kind, name)?;
        let supported = match kind {
            CaptureKind::Microphone => device.default_input_config(),
            #[cfg(windows)]
            CaptureKind::SystemOutput => device.default_output_config(),
            #[cfg(not(windows))]
            CaptureKind::SystemOutput => device.default_input_config(),
        }
        .map_err(|_| CaptureError::UnsupportedFormat)?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let format = AudioFormat {
            sample_rate: config.sample_rate.0,
            channels: config.channels,
        };
        if format.sample_rate == 0 || format.channels == 0 {
            return Err(CaptureError::UnsupportedFormat);
        }
        let capacity = format.sample_rate as usize * usize::from(format.channels) * QUEUE_SECONDS;
        let queue = Arc::new(SampleQueue::new(capacity));
        let stream = match sample_format {
            SampleFormat::F32 => build::<f32>(&device, &config, &queue),
            SampleFormat::I16 => build::<i16>(&device, &config, &queue),
            SampleFormat::U16 => build::<u16>(&device, &config, &queue),
            SampleFormat::I32 => build::<i32>(&device, &config, &queue),
            _ => Err(CaptureError::UnsupportedFormat),
        }?;
        stream.play().map_err(|_| CaptureError::StartFailed)?;
        let label = device.name().unwrap_or_else(|_| "Sound device".to_owned());
        Ok((stream, queue, Opened { label, format }))
    })();
    match opened {
        Err(error) => {
            let _ = reply.send(Err(error));
        }
        Ok((stream, queue, opened)) => {
            let _ = queue_out.send(queue);
            let _ = reply.send(Ok(opened));
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(50));
            }
            drop(stream);
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    queue: &Arc<SampleQueue>,
) -> Result<Stream, CaptureError>
where
    T: SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let data_queue = queue.clone();
    let error_queue = queue.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                data_queue.push(
                    data.iter()
                        .map(|sample| <f32 as cpal::FromSample<T>>::from_sample_(*sample)),
                );
            },
            move |_error| error_queue.fail(),
            None,
        )
        .map_err(|_| CaptureError::StartFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_is_bounded_and_keeps_the_newest_samples() {
        let queue = SampleQueue::new(10);
        queue.push((0..6).map(|n| n as f32));
        queue.push((6..12).map(|n| n as f32));
        let mut out = Vec::new();
        queue.drain_into(&mut out);
        assert_eq!(out, (2..12).map(|n| n as f32).collect::<Vec<_>>());
        // A burst bigger than the queue keeps only its tail.
        queue.push((0..100).map(|n| n as f32));
        out.clear();
        queue.drain_into(&mut out);
        assert_eq!(out, (90..100).map(|n| n as f32).collect::<Vec<_>>());
        assert!(queue.is_empty());
    }

    #[test]
    fn a_failed_device_is_remembered() {
        let queue = SampleQueue::new(4);
        assert!(!queue.failed());
        queue.fail();
        assert!(queue.failed());
    }

    #[test]
    fn opening_a_missing_device_is_an_error_not_a_crash() {
        // Neither a microphone nor a speaker exists in the test environment, or if one does, a
        // bogus name falls back to the default; either way this must return.
        for kind in [CaptureKind::Microphone, CaptureKind::SystemOutput] {
            match DeviceCapture::open(kind, "no such device") {
                Ok(mut capture) => {
                    let mut out = Vec::new();
                    let _ = capture.read(&mut out);
                    assert!(capture.format().sample_rate > 0);
                }
                Err(error) => assert!(!error.to_string().is_empty()),
            }
        }
    }
}
