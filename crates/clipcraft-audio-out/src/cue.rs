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
    /// Clipping began: two soft rising notes.
    ClippingStarted,
    /// Clipping ended: the same two notes falling.
    ClippingStopped,
    /// A clip was saved: a small bright chime.
    ClipSaved,
    /// A screenshot was taken: two soft ticks.
    Screenshot,
    /// The microphone is on: one soft high note.
    MicrophoneOn,
    /// The microphone is off: one soft low note.
    MicrophoneOff,
    /// Something went wrong: two low, gentle falling notes.
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
/// Peak level of a cue at 100 %. Kept low on purpose: the cues are a quiet confirmation, not an
/// alert, and must sit under a game's own sound.
const FULL_LEVEL: f32 = 0.32;
/// The top of the cues' tone, in hertz: everything above is rolled off, which is what makes them
/// soft instead of sharp.
const WARMTH_HERTZ: f32 = 4_200.0;

/// One soft note or tick, placed `at_millis` after the start. Notes overlap, so each one rings
/// into the next like a small chime.
struct Note {
    /// Pitch in hertz; 0 is a short filtered-noise tick.
    hertz: f32,
    at_millis: u32,
    millis: u32,
    gain: f32,
}

fn notes(cue: Cue) -> Vec<Note> {
    let tone = |hertz, at_millis, millis, gain| Note {
        hertz,
        at_millis,
        millis,
        gain,
    };
    match cue {
        // E5 up to A5: a rising fourth, open and friendly.
        Cue::ClippingStarted => vec![tone(659.3, 0, 170, 0.8), tone(880.0, 85, 230, 1.0)],
        // The same two notes falling, a little quieter and shorter.
        Cue::ClippingStopped => vec![tone(880.0, 0, 150, 0.75), tone(659.3, 80, 210, 0.9)],
        // C6 and E6, a bright major third that rings a moment longer.
        Cue::ClipSaved => vec![tone(1046.5, 0, 200, 0.75), tone(1318.5, 75, 300, 1.0)],
        // Two soft ticks, like a shutter heard through a wall.
        Cue::Screenshot => vec![
            tone(0.0, 0, 26, 1.0),
            tone(720.0, 0, 40, 0.35),
            tone(0.0, 62, 22, 0.7),
            tone(540.0, 62, 36, 0.25),
        ],
        Cue::MicrophoneOn => vec![tone(784.0, 0, 130, 1.0)],
        Cue::MicrophoneOff => vec![tone(523.3, 0, 140, 1.0)],
        // Two low, gentle notes falling: noticeable without being a buzz.
        Cue::Problem => vec![tone(329.6, 0, 170, 0.85), tone(246.9, 100, 260, 1.0)],
    }
}

/// The cue as mono samples at `rate` hertz, peaking at `level` (0 to 1) of [`FULL_LEVEL`].
pub fn render(cue: Cue, rate: u32, level: f32) -> Vec<f32> {
    let level = level.clamp(0.0, 1.0) * FULL_LEVEL;
    let per_millisecond = rate as f32 / 1000.0;
    let notes = notes(cue);
    let total = notes
        .iter()
        .map(|note| ((note.at_millis + note.millis) as f32 * per_millisecond) as usize)
        .max()
        .unwrap_or(0);
    let mut mix = vec![0.0_f32; total];
    // A small deterministic generator for the noise in the ticks.
    let mut seed = 0x2545_F491_u32;
    for note in &notes {
        let start = (note.at_millis as f32 * per_millisecond) as usize;
        let length = (note.millis as f32 * per_millisecond) as usize;
        let attack = (rate as f32 * 0.010).max(1.0);
        let release = (length as f32 * 0.35).max(1.0);
        // A pitched note rings (a slow decay); a tick dies almost at once.
        let ring = if note.hertz > 0.0 && note.hertz < 600.0 && length > 0 {
            length as f32 / 3.0
        } else if note.hertz > 0.0 {
            length as f32 / 3.4
        } else {
            length as f32 / 5.0
        };
        let mut smooth = 0.0_f32;
        for index in 0..length {
            let at = index as f32;
            let rise = if at < attack {
                0.5 - 0.5 * (std::f32::consts::PI * at / attack).cos()
            } else {
                1.0
            };
            let fall_start = length as f32 - release;
            let fall = if at > fall_start {
                0.5 + 0.5 * (std::f32::consts::PI * (at - fall_start) / release).cos()
            } else {
                1.0
            };
            let decay = (-at / ring.max(1.0)).exp();
            let wave = if note.hertz <= 0.0 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let noise = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                // Rounded off, so a tick is a soft pat and not a hiss.
                smooth += 0.18 * (noise - smooth);
                smooth * 2.4
            } else {
                let phase = 2.0 * std::f32::consts::PI * note.hertz * at / rate as f32;
                // The note and two quiet overtones: warmer than a bare sine, never buzzy.
                (phase.sin() + 0.2 * (2.0 * phase).sin() + 0.06 * (3.0 * phase).sin()) / 1.26
            };
            if let Some(slot) = mix.get_mut(start + index) {
                *slot += wave * rise * fall * decay * note.gain;
            }
        }
    }
    // Roll off the top, then scale so every cue peaks at the same, quiet level.
    let coefficient = 1.0 - (-2.0 * std::f32::consts::PI * WARMTH_HERTZ / rate as f32).exp();
    let mut warm = 0.0_f32;
    for sample in &mut mix {
        warm += coefficient * (*sample - warm);
        *sample = warm;
    }
    let peak = mix
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    if peak > 0.0 {
        let scale = level / peak;
        for sample in &mut mix {
            *sample *= scale;
        }
    }
    mix
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
    fn the_cues_are_quiet_and_soft_with_little_sharp_high_frequency_content() {
        for cue in Cue::ALL {
            let samples = render(cue, 48_000, 1.0);
            let energy: f32 = samples.iter().map(|s| s * s).sum();
            // The first difference measures how much of the sound is in the highest frequencies.
            let sharp: f32 = samples.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
            assert!(
                sharp < energy * 0.35,
                "{cue:?} is sharp: {sharp} vs {energy}"
            );
            let peak = samples.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
            assert!(peak <= 0.33, "{cue:?} is loud: {peak}");
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
