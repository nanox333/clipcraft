//! Bounded, offline microphone denoising on the recorder worker, never a device callback.

use nnnoiseless::DenoiseState;

use crate::{AudioError, AudioFormat, AudioSource};

const RATE: u32 = 48_000;
const FRAME: usize = DenoiseState::FRAME_SIZE;
const MAX_CHANNELS: u16 = 8;
const MAX_READ_MS: usize = 250;
const PCM_SCALE: f32 = 32_768.0;

/// An opt-in speech noise suppressor around a microphone source.
///
/// The first two input channels are independently resampled and denoised at 48 kHz. Stereo is
/// preserved unless explicitly folded with `with_mono`; a mono source remains mono.
/// Conversion keeps partial interleaved frames and resampling phase across reads. A fixed
/// 480-sample block and the model's one-block signal delay mean sound arrives up to 20 ms later;
/// the recorder's existing 40 ms arrival allowance absorbs this without shifting timestamps.
/// The initial delayed output is discarded, rather than prepended as silence.
///
/// Empty reads do not flush or pad: there is no end-of-stream in a live microphone. Up to 20 ms
/// remains pending, covered by the recorder's arrival allowance. Device loss and oversized reads
/// discard that history so it cannot reappear after recovery. Rates outside 8–192 kHz, more than
/// eight channels, and reads longer than 250 ms are refused with an actionable error.
pub struct NoiseSuppressedSource {
    source: Box<dyn AudioSource>,
    format: AudioFormat,
    states: Vec<Box<DenoiseState<'static>>>,
    channels: usize,
    mono: bool,
    scratch: Vec<f32>,
    partial: [f32; MAX_CHANNELS as usize],
    partial_len: usize,
    previous: Option<[f32; 2]>,
    /// Next output position within the current source interval, in units of 1 / RATE.
    phase: u32,
    frame: [[f32; FRAME]; 2],
    frame_len: usize,
    first: bool,
    worker_warmed: bool,
    /// A local overload can recover without reopening a healthy device.
    local_failure: bool,
}

impl NoiseSuppressedSource {
    /// Wraps a microphone with local speech noise suppression, or explains an unsupported format.
    pub fn new(source: Box<dyn AudioSource>) -> Result<Self, String> {
        Self::with_mono(source, false)
    }

    /// Uses one model for explicitly mono voice, or independent models for stereo voice.
    pub fn with_mono(source: Box<dyn AudioSource>, mono: bool) -> Result<Self, String> {
        let format = source.format();
        Self::validate(format)?;
        let channels = if mono {
            1
        } else {
            usize::from(format.channels.min(2))
        };
        Ok(Self {
            source,
            format,
            states: (0..channels).map(|_| DenoiseState::new()).collect(),
            channels,
            mono,
            scratch: Vec::new(),
            partial: [0.0; MAX_CHANNELS as usize],
            partial_len: 0,
            previous: None,
            phase: 0,
            frame: [[0.0; FRAME]; 2],
            frame_len: 0,
            first: true,
            worker_warmed: false,
            local_failure: false,
        })
    }

    fn validate(format: AudioFormat) -> Result<(), String> {
        if !(8_000..=192_000).contains(&format.sample_rate)
            || !(1..=MAX_CHANNELS).contains(&format.channels)
        {
            return Err(
                "Microphone noise suppression needs an 8–192 kHz device with 1–8 channels. Choose another microphone format or turn noise suppression off."
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.states = (0..self.channels).map(|_| DenoiseState::new()).collect();
        self.partial_len = 0;
        self.partial.fill(0.0);
        self.previous = None;
        self.phase = 0;
        self.frame_len = 0;
        self.frame.fill([0.0; FRAME]);
        self.first = true;
    }

    fn clean(sample: f32) -> f32 {
        if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }

    fn push_sample(&mut self, samples: [f32; 2], out: &mut Vec<f32>) {
        // All values reaching the model are finite signed-16-bit-scale PCM, and both slices
        // passed below always have exactly the fixed size required by its low-level API.
        for (frame, sample) in self.frame.iter_mut().zip(samples).take(self.channels) {
            frame[self.frame_len] = (sample * PCM_SCALE).min(32_767.0);
        }
        self.frame_len += 1;
        if self.frame_len == FRAME {
            let mut processed = [[0.0; FRAME]; 2];
            for (channel, state) in self.states.iter_mut().enumerate() {
                state.process_frame(&mut processed[channel], &self.frame[channel]);
            }
            if self.first {
                self.first = false;
            } else {
                for index in 0..FRAME {
                    for channel in processed.iter().take(self.channels) {
                        out.push(Self::clean(channel[index] / PCM_SCALE));
                    }
                }
            }
            self.frame_len = 0;
        }
    }

    fn push_frame(&mut self, current: [f32; 2], out: &mut Vec<f32>) {
        let Some(previous) = self.previous else {
            self.previous = Some(current);
            self.phase = self.format.sample_rate;
            self.push_sample(current, out);
            return;
        };
        // Integer phase avoids drift and makes conversion independent of device callback sizes.
        // The validated rate bounds this loop to at most six samples per input frame.
        while self.phase <= RATE {
            let fraction = self.phase as f32 / RATE as f32;
            let sample = if self.phase == RATE {
                current
            } else {
                std::array::from_fn(|channel| {
                    previous[channel] + (current[channel] - previous[channel]) * fraction
                })
            };
            self.push_sample(sample, out);
            self.phase += self.format.sample_rate;
        }
        self.phase -= RATE;
        self.previous = Some(current);
    }
}

impl AudioSource for NoiseSuppressedSource {
    fn name(&self) -> String {
        self.source.name()
    }

    fn format(&self) -> AudioFormat {
        AudioFormat {
            sample_rate: RATE,
            channels: self.channels as u16,
        }
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
        if !self.worker_warmed {
            // easyfft initializes worker-local FFT plans and scratch on first use. Do that on
            // this worker with silence, then restore a fresh model before hearing live input.
            let mut ignored = [0.0; FRAME];
            for state in &mut self.states {
                state.process_frame(&mut ignored, &[0.0; FRAME]);
            }
            self.reset();
            self.worker_warmed = true;
        }
        self.scratch.clear();
        if let Err(error) = self.source.read(&mut self.scratch) {
            self.scratch = Vec::new();
            self.local_failure = false;
            self.reset();
            return Err(error);
        }
        let channels = usize::from(self.format.channels);
        // Validation makes all arithmetic small: at most 384,000 incoming samples and 25
        // neural blocks per channel. The device's existing bounded queue owns any backlog; never denoise
        // an arbitrarily long read on the recording worker.
        let max_samples = self.format.sample_rate as usize * MAX_READ_MS / 1_000 * channels;
        if self.scratch.len() > max_samples {
            self.scratch = Vec::new();
            self.local_failure = true;
            self.reset();
            return Err(AudioError::Failed(
                "The microphone fell more than 250 ms behind noise suppression. Recording continues with silence; reduce CPU load or turn noise suppression off."
                    .to_owned(),
            ));
        }
        // Move the reusable allocation aside so processing can borrow the rest of the state.
        let mut incoming = std::mem::take(&mut self.scratch);
        for &sample in &incoming {
            self.partial[self.partial_len] = Self::clean(sample);
            self.partial_len += 1;
            if self.partial_len == channels {
                let left = self.partial[0];
                let right = if channels == 1 { left } else { self.partial[1] };
                let samples = if self.mono {
                    [(left + right) * 0.5; 2]
                } else {
                    [left, right]
                };
                self.push_frame(samples, out);
                self.partial_len = 0;
            }
        }
        incoming.clear();
        self.scratch = incoming;
        Ok(())
    }

    fn recover(&mut self) -> bool {
        if self.local_failure {
            // The recorder waits two seconds between recovery attempts. A healthy device kept
            // filling its queue during that wait, so merely resetting the model would hit the
            // 250 ms limit again forever. Drain that stale backlog once without running DSP.
            self.scratch.clear();
            let drained = self.source.read(&mut self.scratch);
            self.scratch = Vec::new();
            if drained.is_err() {
                // It became a real device failure while waiting: the next attempt must reopen
                // the device instead of repeatedly treating the error as a local overload.
                self.local_failure = false;
                self.reset();
                return false;
            }
        } else if !self.source.recover() {
            return false;
        }
        let format = self.source.format();
        if Self::validate(format).is_err() {
            self.local_failure = false;
            self.reset();
            return false;
        }
        self.format = format;
        self.local_failure = false;
        self.reset();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct Queued {
        format: AudioFormat,
        reads: VecDeque<Result<Vec<f32>, AudioError>>,
        can_recover: bool,
        recovered_format: Option<AudioFormat>,
    }

    impl AudioSource for Queued {
        fn name(&self) -> String {
            "Generated microphone".to_owned()
        }
        fn format(&self) -> AudioFormat {
            self.format
        }
        fn read(&mut self, out: &mut Vec<f32>) -> Result<(), AudioError> {
            if let Some(read) = self.reads.pop_front() {
                out.extend(read?);
            }
            Ok(())
        }
        fn recover(&mut self) -> bool {
            if let Some(format) = self.recovered_format {
                self.format = format;
            }
            self.can_recover
        }
    }

    fn source(rate: u32, channels: u16, reads: Vec<Vec<f32>>) -> Queued {
        Queued {
            format: AudioFormat {
                sample_rate: rate,
                channels,
            },
            reads: reads.into_iter().map(Ok).collect(),
            can_recover: true,
            recovered_format: None,
        }
    }

    fn noise(samples: usize) -> Vec<f32> {
        let mut state = 0x93a5_b7c1_u32;
        (0..samples)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f64 / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * 0.04
            })
            .collect()
    }

    fn run(rate: u32, channels: u16, samples: &[f32], chunk: usize) -> Vec<f32> {
        let reads: Vec<_> = samples.chunks(chunk).map(<[f32]>::to_vec).collect();
        let count = reads.len();
        let mut denoised =
            NoiseSuppressedSource::new(Box::new(source(rate, channels, reads))).unwrap();
        let mut output = Vec::new();
        for _ in 0..count {
            denoised.read(&mut output).unwrap();
        }
        output
    }

    #[test]
    fn unsupported_formats_are_actionable_errors() {
        for (rate, channels) in [(0, 1), (7_999, 1), (192_001, 1), (48_000, 0), (48_000, 9)] {
            let error = NoiseSuppressedSource::new(Box::new(source(rate, channels, vec![])))
                .err()
                .unwrap();
            assert!(error.contains("turn noise suppression off"));
        }
    }

    #[test]
    fn awkward_chunks_and_partial_channel_frames_do_not_change_the_sound_or_length() {
        for (rate, channels) in [
            (8_000, 1),
            (16_000, 2),
            (44_100, 2),
            (48_000, 1),
            (96_000, 4),
            (192_000, 8),
        ] {
            let frames = rate as usize / 10;
            let input = noise(frames * usize::from(channels));
            let whole = run(rate, channels, &input, input.len());
            let split = run(rate, channels, &input, 17);
            assert_eq!(whole, split, "{rate}/{channels}");
            let resampled = (frames - 1) * RATE as usize / rate as usize + 1;
            assert_eq!(
                whole.len(),
                (resampled / FRAME).saturating_sub(1) * FRAME * usize::from(channels.min(2))
            );
            assert!(
                whole
                    .iter()
                    .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
            );
        }
    }

    #[test]
    fn only_complete_blocks_are_emitted_and_the_initial_delayed_block_is_discarded() {
        let mut wrapped = NoiseSuppressedSource::new(Box::new(source(
            RATE,
            1,
            vec![vec![0.0; FRAME - 1], vec![], vec![0.0], vec![0.0; FRAME]],
        )))
        .unwrap();
        let mut out = Vec::new();
        for _ in 0..3 {
            wrapped.read(&mut out).unwrap();
            assert!(out.is_empty());
        }
        wrapped.read(&mut out).unwrap();
        assert_eq!(out, vec![0.0; FRAME]);
        assert_eq!(
            wrapped.format(),
            AudioFormat {
                sample_rate: RATE,
                channels: 1
            }
        );
    }

    #[test]
    fn stereo_channels_match_independent_models_without_crosstalk_or_downmix() {
        let mono = noise(FRAME * 4);
        let stereo: Vec<_> = mono
            .iter()
            .flat_map(|sample| [*sample, -*sample, 1.0, -1.0])
            .collect();
        let actual = run(RATE, 4, &stereo, stereo.len());
        let left = run(RATE, 1, &mono, FRAME);
        let right = run(
            RATE,
            1,
            &mono.iter().map(|sample| -*sample).collect::<Vec<_>>(),
            FRAME,
        );
        let expected: Vec<_> = left
            .iter()
            .zip(&right)
            .flat_map(|(left, right)| [*left, *right])
            .collect();
        assert_eq!(actual, expected);
        assert!(
            actual.iter().any(|sample| sample.abs() > 0.000001),
            "opposite stereo channels must not cancel"
        );
        let mut folded =
            NoiseSuppressedSource::with_mono(Box::new(source(RATE, 4, vec![stereo])), true)
                .unwrap();
        let mut output = Vec::new();
        folded.read(&mut output).unwrap();
        assert_eq!(folded.format().channels, 1);
        assert_eq!(
            output,
            vec![0.0; left.len()],
            "explicit mono should fold before suppression"
        );
    }

    #[test]
    fn a_generated_click_keeps_its_sample_position_instead_of_gaining_a_model_frame_delay() {
        let mut input = vec![0.0; FRAME * 12];
        let click = FRAME * 3 + 173;
        input[click] = 0.8;
        let output = run(RATE, 1, &input, 31);
        let loudest = output
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.abs().total_cmp(&right.abs()))
            .map(|(index, _)| index)
            .unwrap();
        assert!(
            loudest.abs_diff(click) <= 8,
            "click {click}, loudest output {loudest}"
        );
        assert!(output[loudest].abs() > 0.000_001);
    }

    #[test]
    fn the_audio_engine_still_applies_gain_and_mute_after_suppression() {
        use crate::audio::AudioEngine;
        use crate::{AudioConfig, AudioControl, AudioInput, AudioSetup};
        use std::time::Duration;

        fn engine(samples: Vec<f32>, suppressed: bool, control: AudioControl) -> AudioEngine {
            let raw: Box<dyn AudioSource> = Box::new(source(RATE, 1, vec![samples]));
            let input: Box<dyn AudioSource> = if suppressed {
                Box::new(NoiseSuppressedSource::new(raw).unwrap())
            } else {
                raw
            };
            AudioEngine::new(AudioSetup {
                config: AudioConfig {
                    sample_rate: RATE,
                    channels: 1,
                    kbps: 128,
                    tracks: vec!["Microphone".to_owned()],
                },
                inputs: vec![AudioInput::new(input, control, 0)],
            })
            .unwrap()
        }

        let input = noise(FRAME * 20);
        let denoised = run(RATE, 1, &input, FRAME);
        let control = AudioControl::default();
        control.set_gain_percent(50);
        let mut actual = engine(input.clone(), true, control.clone());
        let half: Vec<_> = denoised.iter().map(|sample| sample * 0.5).collect();
        let mut expected = engine(half, false, AudioControl::default());
        assert_eq!(
            actual.tick(Duration::from_millis(200)),
            expected.tick(Duration::from_millis(200))
        );
        assert!(
            control.level() > 0.0,
            "the meter still observes pre-gain audio"
        );

        for muted in [false, true] {
            let control = AudioControl::default();
            control.set_muted(muted);
            if !muted {
                control.set_gain_percent(0);
            }
            let mut actual = engine(input.clone(), true, control.clone());
            let mut silence = engine(vec![0.0; denoised.len()], false, AudioControl::default());
            assert_eq!(
                actual.tick(Duration::from_millis(200)),
                silence.tick(Duration::from_millis(200))
            );
            assert!(
                control.level() > 0.0,
                "mute and zero gain do not suppress the input meter"
            );
        }
    }

    #[test]
    fn non_finite_and_extreme_input_remains_finite_and_bounded() {
        let input: Vec<_> = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MAX,
            -f32::MAX,
            0.0,
        ]
        .into_iter()
        .cycle()
        .take(FRAME * 6)
        .collect();
        let output = run(RATE, 2, &input, 33);
        assert!(!output.is_empty());
        assert!(
            output
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
        );
    }

    #[test]
    fn steady_generated_broadband_noise_is_attenuated_after_warmup() {
        let input = noise(RATE as usize * 2);
        let output = run(RATE, 1, &input, FRAME);
        let rms = |samples: &[f32]| {
            (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32)
                .sqrt()
        };
        let input_rms = rms(&input[RATE as usize..]);
        let output_rms = rms(&output[RATE as usize..]);
        assert!(
            output_rms < input_rms * 0.5,
            "input {input_rms}, output {output_rms}"
        );
    }

    #[test]
    fn loss_discards_partial_samples_and_model_history_before_recovery() {
        let mut queued = source(RATE, 2, vec![]);
        queued.reads = VecDeque::from([
            Ok(vec![0.8; FRAME * 2 + 1]),
            Err(AudioError::DeviceLost("generated unplug".to_owned())),
            Ok(vec![0.0; FRAME * 4]),
        ]);
        let mut wrapped = NoiseSuppressedSource::new(Box::new(queued)).unwrap();
        let mut out = Vec::new();
        wrapped.read(&mut out).unwrap();
        assert!(out.is_empty());
        assert!(wrapped.read(&mut out).is_err());
        assert!(wrapped.recover());
        wrapped.read(&mut out).unwrap();
        assert_eq!(out, vec![0.0; FRAME * 2]);
    }

    #[test]
    fn oversized_reads_fail_without_emitting_or_poisoning_a_healthy_device() {
        let mut queued = source(
            RATE,
            1,
            vec![
                vec![0.5; RATE as usize / 4 + 1],
                // Samples accumulated during the engine's two-second recovery wait.
                vec![0.5; RATE as usize * 2],
                vec![0.0; FRAME * 2],
            ],
        );
        queued.can_recover = false;
        let mut wrapped = NoiseSuppressedSource::new(Box::new(queued)).unwrap();
        let mut out = vec![0.25];
        let error = wrapped.read(&mut out).unwrap_err();
        assert!(error.to_string().contains("250 ms"));
        assert_eq!(out, vec![0.25]);
        assert!(wrapped.recover());
        assert_eq!(wrapped.scratch.capacity(), 0);
        wrapped.read(&mut out).unwrap();
        assert_eq!(&out[1..], &[0.0; FRAME]);
    }

    #[test]
    fn a_device_error_during_overload_recovery_is_reopened_on_the_next_attempt() {
        let mut queued = source(RATE, 1, vec![]);
        queued.reads = VecDeque::from([
            Ok(vec![0.5; RATE as usize / 4 + 1]),
            Err(AudioError::DeviceLost(
                "generated unplug while waiting".to_owned(),
            )),
            Ok(vec![0.0; FRAME * 2]),
        ]);
        let mut wrapped = NoiseSuppressedSource::new(Box::new(queued)).unwrap();
        let mut out = Vec::new();
        assert!(wrapped.read(&mut out).is_err());
        assert!(!wrapped.recover());
        assert!(!wrapped.local_failure);
        assert_eq!(wrapped.scratch.capacity(), 0);
        assert!(wrapped.recover());
        wrapped.read(&mut out).unwrap();
        assert_eq!(out, vec![0.0; FRAME]);
    }

    #[test]
    fn recovery_revalidates_a_changed_device_format() {
        let mut queued = source(RATE, 1, vec![]);
        queued.recovered_format = Some(AudioFormat {
            sample_rate: 1,
            channels: 1,
        });
        let mut wrapped = NoiseSuppressedSource::new(Box::new(queued)).unwrap();
        assert!(!wrapped.recover());
    }

    #[test]
    #[ignore = "release-mode generated workload measurement; no sound hardware"]
    fn generated_noise_suppression_throughput() {
        for channels in [1_u16, 2] {
            let input = noise(RATE as usize * 20 * usize::from(channels));
            let chunk = FRAME * usize::from(channels);
            // Warm the model and the FFT's worker-local scratch before the timed run.
            let _ = run(RATE, channels, &input[..chunk * 4], chunk);
            let started = std::time::Instant::now();
            let output = run(RATE, channels, &input, chunk);
            let elapsed = started.elapsed();
            assert_eq!(output.len(), input.len() - chunk);
            println!(
                "20 s generated 48 kHz/{channels} channels: {:.3} ms total, {:.3} us per 10 ms block, {:.5} x real time",
                elapsed.as_secs_f64() * 1_000.0,
                elapsed.as_secs_f64() * 1_000_000.0 / 2_000.0,
                elapsed.as_secs_f64() / 20.0
            );
        }
    }
}
