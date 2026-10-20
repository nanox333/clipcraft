//! Sound devices by name, and a microphone level meter for testing a microphone.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream, StreamConfig};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Most device names listed, so a hostile driver cannot flood the menu.
pub const MAX_DEVICES: usize = 64;

/// The microphones and speakers the system offers, by name.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeviceNames {
    /// Input devices.
    pub inputs: Vec<String>,
    /// Output devices.
    pub outputs: Vec<String>,
}

/// Lists the sound devices. A host that cannot enumerate gives empty lists.
pub fn list_devices() -> DeviceNames {
    let host = crate::host();
    let collect = |devices: Option<Box<dyn Iterator<Item = cpal::Device>>>| -> Vec<String> {
        let mut names: Vec<String> = devices
            .into_iter()
            .flatten()
            .filter_map(|device| device.name().ok())
            .take(MAX_DEVICES)
            .collect();
        names.sort();
        names.dedup();
        names
    };
    let inputs = catch_unwind(AssertUnwindSafe(|| {
        host.input_devices()
            .ok()
            .map(|devices| Box::new(devices) as Box<dyn Iterator<Item = cpal::Device>>)
    }))
    .unwrap_or(None);
    let outputs = catch_unwind(AssertUnwindSafe(|| {
        host.output_devices()
            .ok()
            .map(|devices| Box::new(devices) as Box<dyn Iterator<Item = cpal::Device>>)
    }))
    .unwrap_or(None);
    DeviceNames {
        inputs: collect(inputs),
        outputs: collect(outputs),
    }
}

/// Why a microphone could not be tested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MicTestError {
    /// No microphone was found.
    NoDevice,
    /// The microphone offers no format ClipCraft can read.
    UnsupportedFormat,
    /// The microphone refused to open or start.
    StartFailed,
}

impl std::fmt::Display for MicTestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoDevice => "No microphone was found.",
            Self::UnsupportedFormat => "The microphone uses a format ClipCraft cannot read.",
            Self::StartFailed => {
                "The microphone could not be started. Check that ClipCraft may use it."
            }
        })
    }
}

impl std::error::Error for MicTestError {}

/// The loudest absolute sample in a block, 0.0 to 1.0.
pub fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |loudest, sample| {
            let magnitude = if sample.is_finite() {
                sample.abs()
            } else {
                0.0
            };
            loudest.max(magnitude)
        })
        .min(1.0)
}

/// A running microphone that reports how loud it is. Dropping it stops the microphone.
pub struct MicMeter {
    _stream: Stream,
    level: Arc<AtomicU32>,
    failed: Arc<AtomicBool>,
}

impl MicMeter {
    /// Opens the named microphone (or the default when `name` is empty) and starts measuring.
    pub fn start(name: &str) -> Result<Self, MicTestError> {
        let host = crate::host();
        let device = if name.is_empty() {
            host.default_input_device()
        } else {
            host.input_devices()
                .ok()
                .and_then(|mut devices| {
                    devices.find(|device| device.name().is_ok_and(|n| n == name))
                })
                .or_else(|| host.default_input_device())
        }
        .ok_or(MicTestError::NoDevice)?;
        let supported = device
            .default_input_config()
            .map_err(|_| MicTestError::UnsupportedFormat)?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let level = Arc::new(AtomicU32::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, &level, &failed),
            SampleFormat::I16 => build::<i16>(&device, &config, &level, &failed),
            SampleFormat::U16 => build::<u16>(&device, &config, &level, &failed),
            _ => return Err(MicTestError::UnsupportedFormat),
        }?;
        stream.play().map_err(|_| MicTestError::StartFailed)?;
        Ok(Self {
            _stream: stream,
            level,
            failed,
        })
    }

    /// The most recent loudness, 0.0 to 1.0.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// True once the device reported an error.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    level: &Arc<AtomicU32>,
    failed: &Arc<AtomicBool>,
) -> Result<Stream, MicTestError>
where
    T: SizedSample + Send + 'static,
    f32: cpal::FromSample<T>,
{
    let level = level.clone();
    let failed = failed.clone();
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                scratch.clear();
                scratch.extend(
                    data.iter()
                        .map(|sample| <f32 as cpal::FromSample<T>>::from_sample_(*sample)),
                );
                let measured = catch_unwind(AssertUnwindSafe(|| peak(&scratch))).unwrap_or(0.0);
                level.store(measured.to_bits(), Ordering::Relaxed);
            },
            move |_error| failed.store(true, Ordering::SeqCst),
            None,
        )
        .map_err(|_| MicTestError::StartFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_is_the_loudest_sample_and_ignores_garbage() {
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(peak(&[0.1, -0.6, 0.3]), 0.6);
        assert_eq!(peak(&[f32::NAN, f32::INFINITY, 0.2]), 0.2);
        assert_eq!(peak(&[5.0]), 1.0);
    }

    #[test]
    fn listing_devices_never_panics_and_is_bounded() {
        let devices = list_devices();
        assert!(devices.inputs.len() <= MAX_DEVICES && devices.outputs.len() <= MAX_DEVICES);
    }
}
