//! Sound device enumeration and sanitized peak measurement.

use cpal::traits::{DeviceTrait, HostTrait};

use std::panic::{AssertUnwindSafe, catch_unwind};

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
