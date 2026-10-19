//! The bounded replay buffer: the most recent encoded video, kept as whole groups of pictures so
//! any saved clip starts on a keyframe.

use std::collections::VecDeque;

/// One encoded picture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedSample {
    /// Presentation time in frames since the recording started. Strictly increasing.
    pub pts: i64,
    /// Length-prefixed H.264 NAL units.
    pub data: Vec<u8>,
    /// True for a picture the stream can start from.
    pub keyframe: bool,
}

/// Why a sample was not stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushError {
    /// The sample's time is not after the previous one.
    OutOfOrder,
    /// The first sample must be a keyframe.
    NotAKeyframe,
}

/// Encoded pictures from the last few seconds, bounded by time and by bytes.
#[derive(Debug)]
pub struct ReplayBuffer {
    samples: VecDeque<EncodedSample>,
    bytes: usize,
    window_frames: i64,
    max_bytes: usize,
    evicted_frames: u64,
}

impl ReplayBuffer {
    /// Keeps at least `window_frames` of the newest video (rounded up to a whole group of
    /// pictures), and never more than about `max_bytes`.
    pub fn new(window_frames: i64, max_bytes: usize) -> Self {
        Self {
            samples: VecDeque::new(),
            bytes: 0,
            window_frames: window_frames.max(1),
            max_bytes: max_bytes.max(1),
            evicted_frames: 0,
        }
    }

    /// Adds the newest picture, then drops the oldest whole groups the window no longer needs.
    pub fn push(&mut self, sample: EncodedSample) -> Result<(), PushError> {
        if self.samples.is_empty() && !sample.keyframe {
            return Err(PushError::NotAKeyframe);
        }
        if self
            .samples
            .back()
            .is_some_and(|last| sample.pts <= last.pts)
        {
            return Err(PushError::OutOfOrder);
        }
        self.bytes = self.bytes.saturating_add(sample.data.len());
        self.samples.push_back(sample);
        self.evict();
        Ok(())
    }

    fn evict(&mut self) {
        loop {
            // Where the second group starts: the first sample after the front that is a keyframe.
            let next_group = self
                .samples
                .iter()
                .skip(1)
                .position(|sample| sample.keyframe)
                .map(|offset| offset + 1);
            let (Some(next_group), Some(last)) = (next_group, self.samples.back()) else {
                break;
            };
            let Some(next) = self.samples.get(next_group) else {
                break;
            };
            let remaining = last.pts - next.pts + 1;
            let over_budget = self.bytes > self.max_bytes;
            if remaining < self.window_frames && !over_budget {
                break;
            }
            for _ in 0..next_group {
                if let Some(old) = self.samples.pop_front() {
                    self.bytes = self.bytes.saturating_sub(old.data.len());
                    self.evicted_frames += 1;
                }
            }
        }
        // One group alone is over budget (a huge keyframe interval): keep memory bounded by
        // starting over at the next keyframe.
        if self.bytes > self.max_bytes.saturating_mul(2) {
            self.evicted_frames += self.samples.len() as u64;
            self.samples.clear();
            self.bytes = 0;
        }
    }

    /// The newest `frames` of video, starting on the keyframe at or before that point so the clip
    /// is at least that long when the buffer holds enough. Empty when nothing is buffered.
    pub fn snapshot(&self, frames: i64) -> Vec<EncodedSample> {
        let Some(last) = self.samples.back() else {
            return Vec::new();
        };
        let wanted_start = last.pts - frames.max(1) + 1;
        let start = self
            .samples
            .iter()
            .rposition(|sample| sample.keyframe && sample.pts <= wanted_start)
            .unwrap_or(0);
        self.samples.iter().skip(start).cloned().collect()
    }

    /// Drops everything, for example when the picture size changes and old video no longer
    /// matches the stream's parameters.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.bytes = 0;
    }

    /// Pictures stored.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True when nothing is stored.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Bytes stored.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Time covered, in frames.
    pub fn span_frames(&self) -> i64 {
        match (self.samples.front(), self.samples.back()) {
            (Some(first), Some(last)) => last.pts - first.pts + 1,
            _ => 0,
        }
    }

    /// Pictures dropped from the front so far.
    pub fn evicted_frames(&self) -> u64 {
        self.evicted_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pts: i64, keyframe: bool) -> EncodedSample {
        EncodedSample {
            pts,
            data: vec![0; 10],
            keyframe,
        }
    }

    /// A stream with a keyframe every `gop` frames.
    fn fill(buffer: &mut ReplayBuffer, frames: i64, gop: i64) {
        for pts in 0..frames {
            buffer.push(sample(pts, pts % gop == 0)).unwrap();
        }
    }

    #[test]
    fn it_keeps_at_least_the_window_in_whole_groups() {
        let mut buffer = ReplayBuffer::new(30, usize::MAX);
        fill(&mut buffer, 200, 10);
        assert!(buffer.span_frames() >= 30, "{}", buffer.span_frames());
        assert!(buffer.span_frames() < 30 + 10, "{}", buffer.span_frames());
        assert!(buffer.snapshot(1_000)[0].keyframe);
        assert_eq!(buffer.snapshot(1_000)[0].pts % 10, 0);
        assert!(buffer.evicted_frames() > 0);
    }

    #[test]
    fn a_snapshot_starts_on_a_keyframe_at_or_before_the_requested_start() {
        let mut buffer = ReplayBuffer::new(100, usize::MAX);
        fill(&mut buffer, 100, 10);
        let clip = buffer.snapshot(25);
        // The newest 25 frames start at pts 75; the keyframe at or before it is 70.
        assert_eq!(clip[0].pts, 70);
        assert_eq!(clip.last().map(|s| s.pts), Some(99));
        // Asking for more than is buffered returns everything.
        assert_eq!(buffer.snapshot(10_000).len(), 100);
    }

    #[test]
    fn the_byte_budget_forces_old_groups_out() {
        let mut buffer = ReplayBuffer::new(10_000, 300);
        fill(&mut buffer, 100, 10);
        assert!(buffer.bytes() <= 300 + 100, "{}", buffer.bytes());
        assert!(buffer.snapshot(10_000)[0].keyframe);
    }

    #[test]
    fn a_single_oversized_group_cannot_grow_without_bound() {
        let mut buffer = ReplayBuffer::new(10, 100);
        buffer.push(sample(0, true)).unwrap();
        for pts in 1..1_000 {
            // After the buffer starts over, pictures are refused until the next keyframe.
            let _ = buffer.push(sample(pts, false));
        }
        assert!(buffer.bytes() <= 300, "{}", buffer.bytes());
    }

    #[test]
    fn bad_order_and_a_missing_first_keyframe_are_refused() {
        let mut buffer = ReplayBuffer::new(10, usize::MAX);
        assert_eq!(buffer.push(sample(0, false)), Err(PushError::NotAKeyframe));
        buffer.push(sample(5, true)).unwrap();
        assert_eq!(buffer.push(sample(5, false)), Err(PushError::OutOfOrder));
        assert_eq!(buffer.push(sample(4, false)), Err(PushError::OutOfOrder));
        assert!(buffer.push(sample(9, false)).is_ok());
    }

    #[test]
    fn an_empty_buffer_yields_an_empty_clip() {
        let buffer = ReplayBuffer::new(10, 100);
        assert!(buffer.snapshot(10).is_empty() && buffer.is_empty());
        assert_eq!(buffer.span_frames(), 0);
    }

    #[test]
    fn clear_forgets_everything() {
        let mut buffer = ReplayBuffer::new(10, usize::MAX);
        fill(&mut buffer, 20, 5);
        buffer.clear();
        assert!(buffer.is_empty() && buffer.bytes() == 0);
        assert!(buffer.push(sample(100, true)).is_ok());
    }
}
