//! Sound output device for ClipCraft playback (layer 5: the only crate that talks to the OS audio
//! system).
//!
//! [`AudioOutput`] opens the default output device, tells the [`AudioFeed`] which rate and channel
//! count the device wants, and pulls sound from the feed in the device callback. The callback
//! never blocks and never panics: if the feed has nothing it writes silence. A device that stops
//! (unplugged, or a driver error) sets [`AudioOutput::failed`] so the caller can restart it, and
//! hands the playback clock back to the wall clock in the meantime.

mod input;

pub use input::{DeviceNames, MAX_DEVICES, MicMeter, MicTestError, list_devices, peak};

use clipcraft_player::audio::AudioFeed;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream, StreamConfig};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Why no sound device could be started.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioOutputError {
    /// There is no output device.
    NoDevice,
    /// The device offers no format ClipCraft can use.
    UnsupportedFormat,
    /// The device refused to open or start.
    StartFailed,
}

impl fmt::Display for AudioOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoDevice => "No sound output device was found. The video plays without sound.",
            Self::UnsupportedFormat => {
                "The sound device uses a format ClipCraft cannot play. The video plays without sound."
            }
            Self::StartFailed => {
                "The sound device could not be started. The video plays without sound."
            }
        })
    }
}

impl std::error::Error for AudioOutputError {}

/// A running output stream pulling from an [`AudioFeed`]. Dropping it stops the sound and hands
/// the clock back to the wall clock.
pub struct AudioOutput {
    // Kept alive for its callback; dropped (and so stopped) with the output.
    _stream: Stream,
    feed: Arc<AudioFeed>,
    failed: Arc<AtomicBool>,
    rate: u32,
    channels: u16,
}

impl AudioOutput {
    /// Opens the default output device and starts pulling from `feed`.
    pub fn start(feed: Arc<AudioFeed>) -> Result<Self, AudioOutputError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(AudioOutputError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|_| AudioOutputError::UnsupportedFormat)?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();

        if !feed.configure_output(config.sample_rate.0, u32::from(config.channels)) {
            return Err(AudioOutputError::UnsupportedFormat);
        }
        let failed = Arc::new(AtomicBool::new(false));
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, &feed, &failed),
            SampleFormat::I16 => build::<i16>(&device, &config, &feed, &failed),
            SampleFormat::U16 => build::<u16>(&device, &config, &feed, &failed),
            _ => return Err(AudioOutputError::UnsupportedFormat),
        }?;
        stream.play().map_err(|_| AudioOutputError::StartFailed)?;
        feed.set_output_active(true);
        Ok(Self {
            _stream: stream,
            feed,
            failed,
            rate: config.sample_rate.0,
            channels: config.channels,
        })
    }

    /// The sample rate and channel count the device is playing.
    pub fn format(&self) -> (u32, u16) {
        (self.rate, self.channels)
    }

    /// True once the device reported an error. The caller should drop this output and start a
    /// new one.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        self.feed.set_output_active(false);
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    feed: &Arc<AudioFeed>,
    failed: &Arc<AtomicBool>,
) -> Result<Stream, AudioOutputError>
where
    T: SizedSample + cpal::FromSample<f32> + Send + 'static,
{
    let pull = feed.clone();
    let on_error_feed = feed.clone();
    let on_error_flag = failed.clone();
    // Scratch buffer reused between callbacks so the callback does not allocate once warmed up.
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                scratch.resize(data.len(), 0.0);
                // A panic in the callback must never unwind into the audio driver.
                let filled = catch_unwind(AssertUnwindSafe(|| pull.fill(&mut scratch))).is_ok();
                for (target, sample) in data.iter_mut().zip(&scratch) {
                    *target = if filled {
                        T::from_sample(*sample)
                    } else {
                        T::from_sample(0.0)
                    };
                }
            },
            move |_error| {
                on_error_flag.store(true, Ordering::SeqCst);
                on_error_feed.set_output_active(false);
            },
            None,
        )
        .map_err(|_| AudioOutputError::StartFailed)
}
