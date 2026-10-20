//! Short sound effects for what just happened: clipping started or stopped, a clip or screenshot
//! saved, the microphone switched, a problem. They are made from plain tones and noise here, so
//! there are no sound files to ship or license.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream, StreamConfig};

/// What a sound effect tells the person.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cue {
    /// Clipping began: two rising notes.
    ClippingStarted,
    /// Clipping ended: two falling notes.
    ClippingStopped,
    /// A clip was saved: a bright two-note chime.
    ClipSaved,
    /// A screenshot was taken: a shutter click.
    Screenshot,
    /// The microphone is on: one high blip.
    MicrophoneOn,
    /// The microphone is off: one low blip.
    MicrophoneOff,
    /// Something went wrong: two low buzzes.
    Problem,
}

impl Cue {
    /// Every cue, for tests and a settings preview.
    pub const ALL: [Cue; 7] = [
        Self::ClippingStarted,
        Self::ClippingStopped,
        Self::ClipSaved,
        Self::Screenshot,
        Self::MicrophoneOn,
        Self::MicrophoneOff,
        Self::Problem,
    ];
}

/// Most cues playing at once; more are skipped rather than piled up.
const MAX_PLAYING: usize = 3;
/// Peak level of a cue at 100 %, well under full scale so it is never harsh.
const FULL_LEVEL: f32 = 0.5;

/// One note: frequency in hertz (0 for noise), length, and loudness from 0 to 1.
struct Note {
    hertz: f32,
    millis: u32,
    gain: f32,
}

fn notes(cue: Cue) -> Vec<Note> {
    let tone = |hertz, millis, gain| Note {
        hertz,
        millis,
        gain,
    };
    match cue {
        Cue::ClippingStarted => vec![tone(660.0, 85, 0.9), tone(880.0, 120, 1.0)],
        Cue::ClippingStopped => vec![tone(880.0, 85, 0.9), tone(587.0, 130, 1.0)],
        Cue::ClipSaved => vec![tone(1046.5, 80, 0.9), tone(1568.0, 170, 1.0)],
        // Two clicks a moment apart: a noise burst with a faint tone, like a shutter.
        Cue::Screenshot => vec![
            tone(0.0, 14, 1.0),
            tone(1800.0, 8, 0.4),
            tone(0.0, 40, 0.0),
            tone(0.0, 12, 0.8),
            tone(1300.0, 8, 0.3),
        ],
        Cue::MicrophoneOn => vec![tone(988.0, 70, 0.9)],
        Cue::MicrophoneOff => vec![tone(494.0, 80, 0.9)],
        Cue::Problem => vec![
            tone(196.0, 130, 0.9),
            tone(0.0, 40, 0.0),
            tone(196.0, 190, 0.9),
        ],
    }
}

/// The cue as mono samples at `rate` hertz, peaking at `level` (0 to 1).
pub fn render(cue: Cue, rate: u32, level: f32) -> Vec<f32> {
    let level = level.clamp(0.0, 1.0) * FULL_LEVEL;
    let mut out = Vec::new();
    // A small deterministic generator for the noise in the shutter click.
    let mut seed = 0x2545_F491_u32;
    for note in notes(cue) {
        let samples = (u64::from(rate) * u64::from(note.millis) / 1000) as usize;
        for index in 0..samples {
            let position = index as f32 / samples.max(1) as f32;
            // A quick fade in, and a decay that ends at silence, so there are no clicks.
            let attack = (index as f32 / (rate as f32 * 0.004)).min(1.0);
            let decay = (1.0 - position).powf(1.6);
            let wave = if note.hertz <= 0.0 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
            } else {
                (2.0 * std::f32::consts::PI * note.hertz * index as f32 / rate as f32).sin()
            };
            out.push(wave * attack * decay * note.gain * level);
        }
    }
    out
}

/// Plays `cue` on the default output at `percent` of its full level, on a thread of its own, and
/// returns at once. Nothing happens, quietly, when there is no sound device.
pub fn play_cue(cue: Cue, percent: u8) {
    static PLAYING: AtomicUsize = AtomicUsize::new(0);
    if percent == 0 || PLAYING.fetch_add(1, Ordering::SeqCst) >= MAX_PLAYING {
        PLAYING.fetch_sub(1, Ordering::SeqCst);
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("clipcraft-cue".to_owned())
        .spawn(move || {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let _ = play(cue, f32::from(percent.min(100)) / 100.0);
            }));
            PLAYING.fetch_sub(1, Ordering::SeqCst);
        });
    if spawned.is_err() {
        PLAYING.fetch_sub(1, Ordering::SeqCst);
    }
}

fn play(cue: Cue, level: f32) -> Option<()> {
    let host = crate::host();
    let device = host.default_output_device()?;
    let supported = device.default_output_config().ok()?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let samples = Arc::new(render(cue, config.sample_rate.0, level));
    let length = samples.len();
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, &samples),
        SampleFormat::I16 => build::<i16>(&device, &config, &samples),
        SampleFormat::U16 => build::<u16>(&device, &config, &samples),
        _ => None,
    }?;
    stream.play().ok()?;
    let seconds = length as f64 / f64::from(config.sample_rate.0);
    // Let it finish, plus a little for the device's own buffer, then close the stream.
    std::thread::sleep(Duration::from_secs_f64(seconds + 0.25));
    drop(stream);
    Some(())
}

fn build<T>(device: &cpal::Device, config: &StreamConfig, samples: &Arc<Vec<f32>>) -> Option<Stream>
where
    T: SizedSample + cpal::FromSample<f32> + Send + 'static,
{
    let channels = usize::from(config.channels).max(1);
    let samples = samples.clone();
    let position = Arc::new(Mutex::new(0_usize));
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    let mut at = position.lock().unwrap_or_else(PoisonError::into_inner);
                    for frame in data.chunks_mut(channels) {
                        let value = samples.get(*at).copied().unwrap_or(0.0);
                        *at += 1;
                        for slot in frame {
                            *slot = T::from_sample(value);
                        }
                    }
                }));
            },
            |_error| {},
            None,
        )
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cue_is_audible_short_clean_and_within_range() {
        for cue in Cue::ALL {
            for rate in [44_100, 48_000, 96_000] {
                let samples = render(cue, rate, 1.0);
                let seconds = samples.len() as f32 / rate as f32;
                assert!((0.05..0.6).contains(&seconds), "{cue:?} lasts {seconds} s");
                let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
                assert!(
                    (0.1..=FULL_LEVEL + 0.001).contains(&peak),
                    "{cue:?} peaks at {peak}"
                );
                assert!(samples.iter().all(|sample| sample.is_finite()));
                // It starts and ends at silence, so there is no click.
                assert!(samples[0].abs() < 0.01, "{cue:?} starts loud");
                assert!(samples[samples.len() - 1].abs() < 0.05, "{cue:?} ends loud");
            }
        }
    }

    #[test]
    fn the_cues_sound_different_and_the_level_scales_them() {
        let all: Vec<Vec<f32>> = Cue::ALL
            .iter()
            .map(|cue| render(*cue, 48_000, 1.0))
            .collect();
        for first in 0..all.len() {
            for second in first + 1..all.len() {
                assert_ne!(all[first], all[second], "cues {first} and {second} match");
            }
        }
        let loud = render(Cue::ClipSaved, 48_000, 1.0);
        let quiet = render(Cue::ClipSaved, 48_000, 0.25);
        let peak = |samples: &[f32]| samples.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
        assert!((peak(&quiet) * 4.0 - peak(&loud)).abs() < 0.01);
        assert!(
            render(Cue::ClipSaved, 48_000, 0.0)
                .iter()
                .all(|s| *s == 0.0)
        );
    }

    #[test]
    fn playing_without_a_sound_device_or_at_zero_volume_never_panics() {
        for cue in Cue::ALL {
            play_cue(cue, 0);
            play_cue(cue, 60);
        }
    }
}
