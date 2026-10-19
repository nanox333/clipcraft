//! Bounded queues for frames and audio packets.
//!
//! Every queue is capped by item count and by bytes, and every item that is refused or evicted is
//! counted, so backpressure is observable instead of silent. Callers pick the overflow policy:
//! preview frames use [`Overflow::DropOldest`]; encoded clip data uses [`Overflow::Reject`] so it
//! is never discarded behind the caller's back.

use std::collections::VecDeque;

/// What a full queue does with a new item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overflow {
    /// Evict the oldest items until the new one fits.
    DropOldest,
    /// Refuse the new item and keep everything already queued.
    Reject,
}

/// Why a push did not enqueue the item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushError {
    /// The item alone is larger than the queue's byte limit.
    TooLarge,
    /// The queue is full and its policy is [`Overflow::Reject`].
    Full,
}

/// Running totals for a queue. Dropped counts include rejected pushes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueStats {
    /// Items accepted.
    pub pushed: u64,
    /// Items handed back by `pop`.
    pub popped: u64,
    /// Items evicted or refused.
    pub dropped_items: u64,
    /// Bytes of evicted or refused items.
    pub dropped_bytes: u64,
}

/// A FIFO queue bounded by item count and total bytes.
#[derive(Debug)]
pub struct BoundedQueue<T> {
    items: VecDeque<(T, usize)>,
    bytes: usize,
    max_items: usize,
    max_bytes: usize,
    policy: Overflow,
    stats: QueueStats,
}

impl<T> BoundedQueue<T> {
    /// Creates a queue. A zero limit makes every push fail with [`PushError::TooLarge`].
    pub fn new(max_items: usize, max_bytes: usize, policy: Overflow) -> Self {
        Self {
            // Never reserve from the caller's limit: it may be hostile or huge.
            items: VecDeque::new(),
            bytes: 0,
            max_items,
            max_bytes,
            policy,
            stats: QueueStats::default(),
        }
    }

    /// Adds `item`, which occupies `size_bytes`, applying the overflow policy.
    pub fn push(&mut self, item: T, size_bytes: usize) -> Result<(), PushError> {
        if size_bytes > self.max_bytes || self.max_items == 0 {
            self.record_drop(size_bytes);
            return Err(PushError::TooLarge);
        }
        while self.is_full_for(size_bytes) {
            match self.policy {
                Overflow::Reject => {
                    self.record_drop(size_bytes);
                    return Err(PushError::Full);
                }
                Overflow::DropOldest => {
                    let Some((_, evicted)) = self.items.pop_front() else {
                        break;
                    };
                    self.bytes = self.bytes.saturating_sub(evicted);
                    self.record_drop(evicted);
                }
            }
        }
        self.bytes = self.bytes.saturating_add(size_bytes);
        self.items.push_back((item, size_bytes));
        self.stats.pushed = self.stats.pushed.saturating_add(1);
        Ok(())
    }

    /// Removes the oldest item.
    pub fn pop(&mut self) -> Option<T> {
        let (item, size) = self.items.pop_front()?;
        self.bytes = self.bytes.saturating_sub(size);
        self.stats.popped = self.stats.popped.saturating_add(1);
        Some(item)
    }

    /// Number of queued items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the queue holds no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Total bytes currently queued.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Running totals, including every drop.
    pub fn stats(&self) -> QueueStats {
        self.stats
    }

    fn is_full_for(&self, size_bytes: usize) -> bool {
        self.items.len() >= self.max_items
            // An overflowing total is over any limit; saturating would clamp it to the limit.
            || self
                .bytes
                .checked_add(size_bytes)
                .is_none_or(|total| total > self.max_bytes)
    }

    fn record_drop(&mut self, size_bytes: usize) {
        self.stats.dropped_items = self.stats.dropped_items.saturating_add(1);
        self.stats.dropped_bytes = self
            .stats
            .dropped_bytes
            .saturating_add(u64::try_from(size_bytes).unwrap_or(u64::MAX));
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundedQueue, Overflow, PushError};

    /// Deterministic pseudo-random sequence so failures reproduce exactly.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }
    }

    #[test]
    fn drop_oldest_evicts_in_order_and_counts_every_drop() {
        let mut queue = BoundedQueue::new(3, 100, Overflow::DropOldest);
        for frame in 0..5_u32 {
            queue.push(frame, 10).expect("fits after eviction");
        }
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.pop(), Some(2));
        let stats = queue.stats();
        assert_eq!(
            (stats.pushed, stats.dropped_items, stats.dropped_bytes),
            (5, 2, 20)
        );
    }

    #[test]
    fn reject_keeps_queued_data_and_reports_the_refusal() {
        let mut queue = BoundedQueue::new(2, 100, Overflow::Reject);
        queue.push("a", 40).expect("first");
        queue.push("b", 40).expect("second");
        assert_eq!(queue.push("c", 10), Err(PushError::Full));
        assert_eq!(queue.push("big", 101), Err(PushError::TooLarge));
        assert_eq!(queue.pop(), Some("a"));
        assert_eq!(queue.pop(), Some("b"));
        assert_eq!(queue.stats().dropped_items, 2);
    }

    #[test]
    fn byte_limit_applies_even_when_the_item_limit_is_not_reached() {
        let mut queue = BoundedQueue::new(1_000, 100, Overflow::DropOldest);
        queue.push(1, 60).expect("first");
        queue.push(2, 60).expect("evicts first");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.bytes(), 60);
    }

    #[test]
    fn zero_limits_and_extreme_sizes_never_panic_or_grow() {
        let mut empty = BoundedQueue::new(0, 100, Overflow::DropOldest);
        assert_eq!(empty.push(1, 1), Err(PushError::TooLarge));
        let mut tiny = BoundedQueue::new(4, 0, Overflow::Reject);
        assert_eq!(tiny.push(1, 1), Err(PushError::TooLarge));
        let mut huge = BoundedQueue::new(usize::MAX, usize::MAX, Overflow::DropOldest);
        huge.push(1, usize::MAX).expect("one maximal item fits");
        huge.push(2, usize::MAX)
            .expect("evicts the first without overflow");
        assert_eq!(huge.len(), 1);
    }

    #[test]
    fn random_workloads_never_exceed_limits_and_account_for_every_item() {
        for (seed, policy) in [(1_u64, Overflow::DropOldest), (2, Overflow::Reject)] {
            let mut random = Lcg(seed);
            let mut queue = BoundedQueue::new(16, 4_096, policy);
            let mut attempted = 0_u64;
            for _ in 0..50_000 {
                if random.next(3) == 0 {
                    let _ = queue.pop();
                } else {
                    attempted += 1;
                    let _ = queue.push(attempted, random.next(1_500) as usize);
                }
                assert!(queue.len() <= 16, "item limit exceeded");
                assert!(queue.bytes() <= 4_096, "byte limit exceeded");
            }
            let stats = queue.stats();
            assert_eq!(
                stats.popped + stats.dropped_items + queue.len() as u64,
                attempted,
                "every attempted item is popped, dropped, or still queued"
            );
        }
    }
}
