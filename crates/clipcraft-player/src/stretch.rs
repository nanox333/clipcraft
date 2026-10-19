//! Pitch-preserving speed change (WSOLA: waveform-similarity overlap-add).
//!
//! The clip is read at a nominal pace of `speed` source samples per output sample. Short Hann
//! windowed segments are cut from the source, each nudged a little to line up with the natural
//! continuation of the previous one, and overlap-added at a fixed output hop. Pitch is untouched
//! because no sample is resampled; only how much source each output hop consumes changes.
//! Speed 1.0 is never routed through here, so normal playback stays bit-exact.

/// Slowest and fastest supported speeds.
pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;

/// Window length in seconds (long enough for low pitches, short enough to stay transparent).
const WINDOW_SECONDS: f64 = 0.024;
/// Source samples kept behind the current position, beyond what the next search needs.
const KEEP_BEHIND: usize = 64;

/// Supplies `frames` planar source samples starting at media sample `start`.
pub trait Source {
    /// Returns planar samples (any channel count), or `None` if the source failed.
    fn fetch(&mut self, start: i64, frames: usize) -> Option<Vec<Vec<f32>>>;
}

impl<F: FnMut(i64, usize) -> Option<Vec<Vec<f32>>>> Source for F {
    fn fetch(&mut self, start: i64, frames: usize) -> Option<Vec<Vec<f32>>> {
        self(start, frames)
    }
}

/// Streaming time-scale modifier producing `hop` output frames per call.
pub struct Stretcher {
    speed: f64,
    window: Vec<f32>,
    hop: usize,
    tolerance: usize,
    channels: usize,
    /// Planar source samples; `input[c][0]` is media sample `origin`.
    input: Vec<Vec<f32>>,
    origin: i64,
    /// Where the next segment is nominally taken from, in media samples.
    nominal: f64,
    /// Where the previous segment started, once there is one.
    previous: Option<i64>,
    /// The windowed second half of the previous segment, waiting to be added to the next block.
    overlap: Vec<Vec<f32>>,
    /// Media sample after which there is nothing left to read.
    end: i64,
}

impl Stretcher {
    /// Starts at media sample `start`. `rate` is the sample rate of both source and output;
    /// `end` is the media sample count (reading stops there).
    pub fn new(rate: u32, speed: f64, start: i64, end: i64) -> Self {
        let speed = if speed.is_finite() {
            speed.clamp(MIN_SPEED, MAX_SPEED)
        } else {
            1.0
        };
        let half = ((f64::from(rate.max(1)) * WINDOW_SECONDS / 2.0).round() as usize).max(8);
        let length = half * 2;
        let window = (0..length)
            .map(|n| {
                let phase = std::f64::consts::PI * (n as f64 + 0.5) / length as f64;
                (phase.sin() * phase.sin()) as f32
            })
            .collect();
        Self {
            speed,
            window,
            hop: half,
            tolerance: half / 2,
            channels: 0,
            input: Vec::new(),
            origin: start.max(0),
            nominal: start.max(0) as f64,
            previous: None,
            overlap: Vec::new(),
            end: end.max(0),
        }
    }

    /// Output frames produced per call to [`Stretcher::next_block`].
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Whether the nominal read position has passed the end of the media.
    pub fn finished(&self) -> bool {
        self.nominal >= self.end as f64
    }

    /// One source sample by media index; silence outside the buffered range.
    fn sample(&self, channel: usize, media_index: i64) -> f32 {
        usize::try_from(media_index - self.origin)
            .ok()
            .and_then(|index| self.input.get(channel)?.get(index).copied())
            .unwrap_or(0.0)
    }

    fn mono(&self, from: i64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|offset| {
                let index = from + offset as i64 - self.origin;
                let mut sum = 0.0;
                for channel in &self.input {
                    if let Some(sample) = usize::try_from(index).ok().and_then(|i| channel.get(i)) {
                        sum += *sample;
                    }
                }
                sum / self.channels.max(1) as f32
            })
            .collect()
    }

    /// Makes sure source samples up to (not including) `until` are buffered.
    fn ensure(&mut self, source: &mut impl Source, until: i64) -> Option<()> {
        let until = until.min(self.end.saturating_add(self.window.len() as i64));
        loop {
            let have_until = self.origin + self.input.first().map_or(0, Vec::len) as i64;
            if have_until >= until {
                return Some(());
            }
            let want = usize::try_from(until - have_until)
                .ok()?
                .clamp(1024, 16_384);
            let block = source.fetch(have_until.max(self.origin), want)?;
            if block.is_empty() {
                return None;
            }
            if self.input.is_empty() {
                self.channels = block.len();
                self.input = vec![Vec::new(); block.len()];
                self.overlap = vec![vec![0.0; self.hop]; block.len()];
            }
            let mut added = 0;
            for (channel, samples) in self.input.iter_mut().zip(&block) {
                added = added.max(samples.len());
                channel.extend_from_slice(samples);
                // Channels that came back short are padded so all stay the same length.
                if samples.len() < want {
                    channel.resize(channel.len() + (want - samples.len()), 0.0);
                }
            }
            if added == 0 {
                return None;
            }
        }
    }

    /// Produces the next `hop` frames (planar), or `None` when the source failed. Callers stop
    /// once [`Stretcher::finished`] is true.
    pub fn next_block(&mut self, source: &mut impl Source) -> Option<Vec<Vec<f32>>> {
        let hop = self.hop;
        let length = self.window.len();
        let tolerance = self.tolerance as i64;
        let target = self.nominal.round() as i64;
        self.ensure(source, target + tolerance + length as i64 + hop as i64 + 2)?;

        let chosen = match self.previous {
            None => target.max(0),
            Some(previous) => {
                let template = self.mono(previous + hop as i64, hop);
                let low = (target - tolerance).max(0);
                let high = target + tolerance;
                let mut best = (low, f32::NEG_INFINITY);
                let mut candidate = low;
                // Coarse search on every other offset, then refine around the winner.
                while candidate <= high {
                    let score = self.score(&template, candidate);
                    if score > best.1 {
                        best = (candidate, score);
                    }
                    candidate += 2;
                }
                for refine in [best.0 - 1, best.0 + 1] {
                    if (low..=high).contains(&refine) {
                        let score = self.score(&template, refine);
                        if score > best.1 {
                            best = (refine, score);
                        }
                    }
                }
                best.0
            }
        };

        let first = self.previous.is_none();
        let mut block = Vec::with_capacity(self.channels);
        for channel in 0..self.channels {
            // The very first block is taken unwindowed so the clip does not fade in.
            let head: Vec<f32> = (0..hop)
                .map(|n| {
                    let gain = if first { 1.0 } else { self.window[n] };
                    let carried = self.overlap[channel][n];
                    carried + self.sample(channel, chosen + n as i64) * gain
                })
                .collect();
            let tail: Vec<f32> = (0..hop)
                .map(|n| self.sample(channel, chosen + (hop + n) as i64) * self.window[hop + n])
                .collect();
            self.overlap[channel] = tail;
            block.push(head);
        }
        self.previous = Some(chosen);
        self.nominal += hop as f64 * self.speed;

        // Drop source samples that can no longer be needed.
        let keep_from = (chosen - self.tolerance as i64 - KEEP_BEHIND as i64).max(self.origin);
        let drop = usize::try_from(keep_from - self.origin).unwrap_or(0);
        if drop > 8_192 {
            for channel in &mut self.input {
                channel.drain(..drop.min(channel.len()));
            }
            self.origin += drop as i64;
        }
        Some(block)
    }

    /// Normalised similarity of `template` to the source starting at `candidate` (every other
    /// sample, which is plenty to find a waveform alignment).
    fn score(&self, template: &[f32], candidate: i64) -> f32 {
        let mut dot = 0.0_f32;
        let mut energy = 1e-9_f32;
        let base = candidate - self.origin;
        for (n, wanted) in template.iter().enumerate().step_by(2) {
            let sample = usize::try_from(base + n as i64).map_or(0.0, |index| {
                let sum: f32 = self
                    .input
                    .iter()
                    .map(|channel| channel.get(index).copied().unwrap_or(0.0))
                    .sum();
                sum / self.channels.max(1) as f32
            });
            dot += wanted * sample;
            energy += sample * sample;
        }
        dot / energy.sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A mono 440 Hz sine of `seconds` length, as a source.
    fn tone(seconds: f64, hertz: f64) -> impl FnMut(i64, usize) -> Option<Vec<Vec<f32>>> {
        let total = (seconds * f64::from(RATE)) as i64;
        move |start, frames| {
            let samples: Vec<f32> = (0..frames as i64)
                .map(|n| {
                    let index = start + n;
                    if (0..total).contains(&index) {
                        (index as f64 * hertz * std::f64::consts::TAU / f64::from(RATE)).sin()
                            as f32
                            * 0.5
                    } else {
                        0.0
                    }
                })
                .collect();
            Some(vec![samples])
        }
    }

    /// Runs the stretcher to the end of a `seconds`-long source and returns the mono output.
    fn render(seconds: f64, speed: f64, hertz: f64) -> Vec<f32> {
        let total = (seconds * f64::from(RATE)) as i64;
        let mut stretcher = Stretcher::new(RATE, speed, 0, total);
        let mut source = tone(seconds, hertz);
        let mut out = Vec::new();
        while !stretcher.finished() {
            let block = stretcher.next_block(&mut source).expect("block");
            out.extend_from_slice(&block[0]);
        }
        out
    }

    fn pitch(samples: &[f32]) -> f64 {
        // Skip the first and last 50 ms, where the windows ramp.
        let trim = (f64::from(RATE) * 0.05) as usize;
        let body = &samples[trim..samples.len() - trim];
        let crossings = body
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count();
        crossings as f64 / (body.len() as f64 / f64::from(RATE))
    }

    #[test]
    fn faster_playback_is_shorter_but_keeps_its_pitch() {
        for speed in [1.5, 2.0, 4.0] {
            let out = render(2.0, speed, 440.0);
            let seconds = out.len() as f64 / f64::from(RATE);
            assert!(
                (seconds - 2.0 / speed).abs() < 0.08,
                "{speed}x lasted {seconds} s"
            );
            let measured = pitch(&out);
            assert!(
                (measured - 440.0).abs() < 12.0,
                "{speed}x pitch was {measured} Hz"
            );
        }
    }

    #[test]
    fn slower_playback_is_longer_but_keeps_its_pitch() {
        for speed in [0.25, 0.5, 0.75] {
            let out = render(1.0, speed, 440.0);
            let seconds = out.len() as f64 / f64::from(RATE);
            assert!(
                (seconds - 1.0 / speed).abs() < 0.08,
                "{speed}x lasted {seconds} s"
            );
            let measured = pitch(&out);
            assert!(
                (measured - 440.0).abs() < 12.0,
                "{speed}x pitch was {measured} Hz"
            );
        }
    }

    #[test]
    fn low_pitches_survive_the_window_length() {
        let out = render(2.0, 2.0, 110.0);
        let measured = pitch(&out);
        assert!((measured - 110.0).abs() < 8.0, "pitch was {measured} Hz");
    }

    #[test]
    fn output_stays_in_range_without_clicks_in_the_middle() {
        let out = render(2.0, 1.75, 440.0);
        assert!(
            out.iter().all(|s| s.is_finite() && s.abs() <= 0.6),
            "peak {}",
            out.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
        );
        // A sine's neighbouring samples never differ much; a splice error would show as a jump.
        let trim = (f64::from(RATE) * 0.05) as usize;
        let biggest_step = out[trim..out.len() - trim]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            biggest_step < 0.2,
            "largest sample-to-sample jump {biggest_step}"
        );
    }

    #[test]
    fn silence_stays_silent_and_start_offsets_are_honoured() {
        let mut stretcher = Stretcher::new(RATE, 2.0, 0, 48_000);
        let mut silent = |_: i64, frames: usize| Some(vec![vec![0.0_f32; frames]]);
        let block = stretcher.next_block(&mut silent).expect("block");
        assert!(block[0].iter().all(|s| *s == 0.0));

        // Starting at 1.0 s of a tone reads from there: same pitch, no read before the start.
        let mut source = {
            let mut inner = tone(2.0, 440.0);
            move |start: i64, frames: usize| {
                assert!(start >= 48_000, "read before the start at {start}");
                inner(start, frames)
            }
        };
        let mut late = Stretcher::new(RATE, 1.5, 48_000, 96_000);
        let mut out = Vec::new();
        while !late.finished() {
            out.extend_from_slice(&late.next_block(&mut source).expect("block")[0]);
        }
        assert!((pitch(&out) - 440.0).abs() < 12.0);
    }

    #[test]
    fn hostile_speeds_rates_and_sources_never_panic() {
        for speed in [f64::NAN, f64::INFINITY, -3.0, 0.0, 1e12, f64::MIN_POSITIVE] {
            let mut stretcher = Stretcher::new(RATE, speed, 0, 10_000);
            let mut source = tone(1.0, 440.0);
            for _ in 0..20 {
                if stretcher.next_block(&mut source).is_none() {
                    break;
                }
            }
        }
        // Absurd sample rates and negative bounds are clamped, not trusted.
        let mut odd = Stretcher::new(0, 2.0, -5, -10);
        assert!(odd.finished());
        let huge = Stretcher::new(u32::MAX, 2.0, 0, 100);
        assert!(huge.hop() > 0);
        let _ = odd.next_block(&mut tone(1.0, 440.0));
        // A source that fails or returns nothing ends the stream cleanly.
        let mut stretcher = Stretcher::new(RATE, 2.0, 0, 48_000);
        assert!(stretcher.next_block(&mut |_: i64, _: usize| None).is_none());
        let mut stretcher = Stretcher::new(RATE, 2.0, 0, 48_000);
        assert!(
            stretcher
                .next_block(&mut |_: i64, _: usize| Some(Vec::new()))
                .is_none()
        );
        // Mono, stereo and uneven channel lengths all work.
        let mut stretcher = Stretcher::new(RATE, 2.0, 0, 48_000);
        let mut uneven =
            |_: i64, frames: usize| Some(vec![vec![0.1; frames], vec![0.1; frames / 2]]);
        let block = stretcher.next_block(&mut uneven).expect("block");
        assert_eq!(block.len(), 2);
        assert!(block.iter().all(|c| c.len() == stretcher.hop()));
    }
}
