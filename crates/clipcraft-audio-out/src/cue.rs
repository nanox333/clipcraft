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
/// Peak level of a cue at 100 %. Low on purpose: the cues are a quiet confirmation, not an alert,
/// and must sit under a game's own sound.
const FULL_LEVEL: f32 = 0.30;

/// What makes the sound of one voice.
#[derive(Clone, Copy)]
enum Voice {
    /// A struck glass or soft bell: the pitch plus a few quieter, faster-fading overtones that are
    /// not whole multiples of it, which is what makes a tone sound struck instead of electronic.
    Bell,
    /// A rounder, woodier tone for the low "problem" and microphone-off notes: mostly the pitch,
    /// one soft octave overtone, a faster fade.
    Wood,
    /// A short filtered-noise tap for the screenshot.
    Tap,
}

/// One sound in a cue: when it starts, how long it rings, how loud, and how its pitch moves.
struct Note {
    voice: Voice,
    hertz: f32,
    /// Pitch the note starts from; it slides to `hertz` over its first moments (0 = no slide).
    from_hertz: f32,
    at_millis: u32,
    millis: u32,
    gain: f32,
}

fn notes(cue: Cue) -> Vec<Note> {
    let note = |voice, hertz, at_millis, millis, gain| Note {
        voice,
        hertz,
        from_hertz: 0.0,
        at_millis,
        millis,
        gain,
    };
    let bell = |hertz, at, millis, gain| note(Voice::Bell, hertz, at, millis, gain);
    let wood = |hertz, at, millis, gain| note(Voice::Wood, hertz, at, millis, gain);
    match cue {
        // G5 then D6, a rising fifth: open and confident, like something switching on.
        Cue::ClippingStarted => vec![bell(784.0, 0, 190, 0.8), bell(1174.7, 95, 280, 1.0)],
        // The same fifth falling and a little softer: something settling to rest.
        Cue::ClippingStopped => vec![bell(1174.7, 0, 170, 0.7), bell(784.0, 90, 260, 0.85)],
        // C6, E6, G6: a quick rising major arpeggio that rings out. The reward sound.
        Cue::ClipSaved => vec![
            bell(1046.5, 0, 200, 0.7),
            bell(1318.5, 70, 220, 0.85),
            bell(1568.0, 140, 330, 1.0),
        ],
        // A soft shutter: two taps with a low body under each.
        Cue::Screenshot => vec![
            note(Voice::Tap, 2000.0, 0, 30, 1.0),
            wood(170.0, 0, 45, 0.4),
            note(Voice::Tap, 1600.0, 70, 26, 0.7),
            wood(130.0, 70, 40, 0.3),
        ],
        // A small upward blip and a small downward one.
        Cue::MicrophoneOn => vec![Note {
            from_hertz: 698.5,
            ..bell(1046.5, 0, 150, 1.0)
        }],
        Cue::MicrophoneOff => vec![Note {
            from_hertz: 698.5,
            ..wood(523.3, 0, 160, 1.0)
        }],
        // Two low, round notes falling: clear, but nothing like an alarm.
        Cue::Problem => vec![wood(329.6, 0, 190, 0.9), wood(220.0, 110, 300, 1.0)],
    }
}

/// A very small room: a few decaying echoes, mixed in quietly, so the notes have a little space
/// around them instead of sounding like bare computer tones.
fn add_room(dry: &[f32], rate: u32) -> Vec<f32> {
    let per_millisecond = rate as f32 / 1000.0;
    let tail = (90.0 * per_millisecond) as usize;
    let mut wet = vec![0.0_f32; dry.len() + tail];
    wet[..dry.len()].copy_from_slice(dry);
    // (delay in ms, gain): unrelated delays so the echoes do not ring at one pitch.
    for (milliseconds, gain) in [(23.0, 0.22), (37.0, 0.16), (53.0, 0.11), (71.0, 0.07)] {
        let delay = (milliseconds * per_millisecond) as usize;
        for (index, sample) in dry.iter().enumerate() {
            if let Some(slot) = wet.get_mut(index + delay) {
                *slot += sample * gain;
            }
        }
    }
    wet
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
    let mut dry = vec![0.0_f32; total];
    // A small deterministic generator for the taps.
    let mut seed = 0x2545_F491_u32;
    let two_pi = 2.0 * std::f32::consts::PI;
    for note in &notes {
        let start = (note.at_millis as f32 * per_millisecond) as usize;
        let length = (note.millis as f32 * per_millisecond) as usize;
        let attack = (rate as f32 * 0.004).max(1.0);
        let release = (length as f32 * 0.3).max(1.0);
        let seconds = note.millis as f32 / 1000.0;
        // (frequency ratio, loudness, how fast it fades compared with the pitch).
        let partials: &[(f32, f32, f32)] = match note.voice {
            Voice::Bell => &[
                (1.0, 1.0, 1.0),
                (2.0, 0.28, 2.2),
                (2.76, 0.16, 3.4),
                (5.4, 0.06, 6.0),
            ],
            Voice::Wood => &[(1.0, 1.0, 1.0), (2.0, 0.18, 2.5), (3.0, 0.05, 4.0)],
            Voice::Tap => &[],
        };
        let base_decay = match note.voice {
            Voice::Bell => seconds / 3.2,
            Voice::Wood => seconds / 4.0,
            Voice::Tap => seconds / 5.0,
        };
        let mut phase = 0.0_f32;
        let mut tap = 0.0_f32;
        let mut previous = 0.0_f32;
        for index in 0..length {
            let at = index as f32;
            let time = at / rate as f32;
            let rise = 0.5 - 0.5 * (std::f32::consts::PI * (at / attack).min(1.0)).cos();
            let fall_start = length as f32 - release;
            let fall = if at > fall_start {
                0.5 + 0.5 * (std::f32::consts::PI * (at - fall_start) / release).cos()
            } else {
                1.0
            };
            let wave = match note.voice {
                Voice::Tap => {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let noise = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                    // Band-pass around the tap's pitch: a low-pass for the body, minus a slower
                    // low-pass, leaves a papery click rather than a hiss.
                    let fast = 1.0 - (-two_pi * note.hertz / rate as f32).exp();
                    let slow = 1.0 - (-two_pi * note.hertz * 0.25 / rate as f32).exp();
                    tap += fast * (noise - tap);
                    previous += slow * (noise - previous);
                    (tap - previous) * 2.2 * (-time / base_decay).exp()
                }
                _ => {
                    // The pitch slides to its target over the first 40 ms when it has a start.
                    let glide = if note.from_hertz > 0.0 {
                        (time / 0.04).min(1.0)
                    } else {
                        1.0
                    };
                    let hertz = note.from_hertz + (note.hertz - note.from_hertz) * glide;
                    let hertz = if note.from_hertz > 0.0 {
                        hertz
                    } else {
                        note.hertz
                    };
                    phase += two_pi * hertz / rate as f32;
                    let mut sum = 0.0;
                    let mut weight = 0.0;
                    for (ratio, loudness, fade) in partials {
                        let envelope = (-time * fade / base_decay).exp();
                        sum += (phase * ratio).sin() * loudness * envelope;
                        weight += loudness;
                    }
                    sum / weight.max(1.0)
                }
            };
            if let Some(slot) = dry.get_mut(start + index) {
                *slot += wave * rise * fall * note.gain;
            }
        }
    }
    let mut mix = add_room(&dry, rate);
    // End on silence, smoothly, whatever the echoes left.
    let fade = ((25.0 * per_millisecond) as usize).min(mix.len());
    let length = mix.len();
    for (index, sample) in mix.iter_mut().enumerate().skip(length - fade) {
        let position = (index - (length - fade)) as f32 / fade.max(1) as f32;
        *sample *= 0.5 + 0.5 * (std::f32::consts::PI * position).cos();
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
