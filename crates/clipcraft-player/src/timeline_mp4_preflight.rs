//! Read bounded MP4 index bytes and reject expanding sample tables before the demuxer allocates.

use super::{TimelineError, check_cancel};
use filmcraft_isobmff::ByteSource;
use std::path::Path;
use std::sync::atomic::AtomicBool;

const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BOXES: u32 = 100_000;
const MAX_DEPTH: u8 = 8;
const MAX_DECLARED_SAMPLES: u64 = 1_000_000;
const MAX_ENCODED_SAMPLE: u32 = 64 * 1024 * 1024;

fn word(payload: &[u8], offset: usize) -> Result<u32, TimelineError> {
    let end = offset.checked_add(4).ok_or(TimelineError::DecodeFailed)?;
    let bytes: [u8; 4] = payload
        .get(offset..end)
        .and_then(|v| v.try_into().ok())
        .ok_or(TimelineError::DecodeFailed)?;
    Ok(u32::from_be_bytes(bytes))
}

fn encoded_size(size: u32) -> Result<(), TimelineError> {
    if size > MAX_ENCODED_SAMPLE {
        Err(TimelineError::Unsupported)
    } else {
        Ok(())
    }
}

struct Header {
    kind: [u8; 4],
    size: u64,
    header: u64,
}

fn header(
    source: &(impl ByteSource + ?Sized),
    start: u64,
    end: u64,
) -> Result<Header, TimelineError> {
    if end.checked_sub(start).is_none_or(|n| n < 8) {
        return Err(TimelineError::DecodeFailed);
    }
    let mut bytes = [0_u8; 16];
    source
        .read_at(start, &mut bytes[..8])
        .map_err(|_| TimelineError::Unreadable)?;
    let declared = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let kind = [bytes[4], bytes[5], bytes[6], bytes[7]];
    let (size, header) = match declared {
        0 => (end - start, 8),
        1 => {
            if end - start < 16 {
                return Err(TimelineError::DecodeFailed);
            }
            source
                .read_at(start + 8, &mut bytes[8..])
                .map_err(|_| TimelineError::Unreadable)?;
            (
                u64::from_be_bytes([
                    bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                    bytes[15],
                ]),
                16,
            )
        }
        size => (u64::from(size), 8),
    };
    if size < header || start.checked_add(size).is_none_or(|next| next > end) {
        return Err(TimelineError::DecodeFailed);
    }
    Ok(Header { kind, size, header })
}

#[derive(Default)]
struct Budget {
    boxes: u32,
    samples: u64,
}
impl Budget {
    fn box_seen(&mut self, cancel: &AtomicBool) -> Result<(), TimelineError> {
        check_cancel(cancel)?;
        self.boxes = self
            .boxes
            .checked_add(1)
            .filter(|&n| n <= MAX_BOXES)
            .ok_or(TimelineError::Unsupported)?;
        Ok(())
    }
    fn samples_seen(&mut self, payload: &[u8], offset: usize) -> Result<(), TimelineError> {
        self.samples = self
            .samples
            .checked_add(u64::from(word(payload, offset)?))
            .filter(|&n| n <= MAX_DECLARED_SAMPLES)
            .ok_or(TimelineError::Unsupported)?;
        Ok(())
    }
}

fn encoded_table(
    payload: &[u8],
    start: usize,
    count: u32,
    stride: usize,
    cancel: &AtomicBool,
) -> Result<(), TimelineError> {
    if count == 0 {
        return Ok(());
    }
    let last = (count as usize - 1)
        .checked_mul(stride)
        .and_then(|n| n.checked_add(start))
        .and_then(|n| n.checked_add(4))
        .ok_or(TimelineError::DecodeFailed)?;
    if last > payload.len() {
        return Err(TimelineError::DecodeFailed);
    }
    for i in 0..count as usize {
        if i.is_multiple_of(1024) {
            check_cancel(cancel)?;
        }
        encoded_size(word(payload, start + i * stride)?)?;
    }
    Ok(())
}

fn children(
    bytes: &[u8],
    depth: u8,
    budget: &mut Budget,
    cancel: &AtomicBool,
) -> Result<(), TimelineError> {
    if depth > MAX_DEPTH {
        return Err(TimelineError::Unsupported);
    }
    let mut at = 0_u64;
    while at < bytes.len() as u64 {
        budget.box_seen(cancel)?;
        let h = header(bytes, at, bytes.len() as u64)?;
        let start = usize::try_from(at + h.header).map_err(|_| TimelineError::Unsupported)?;
        let end = usize::try_from(at + h.size).map_err(|_| TimelineError::Unsupported)?;
        let payload = bytes.get(start..end).ok_or(TimelineError::DecodeFailed)?;
        match &h.kind {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"mvex" | b"moof" | b"traf" => {
                children(payload, depth + 1, budget, cancel)?
            }
            b"stsz" => {
                budget.samples_seen(payload, 8)?;
                let size = word(payload, 4)?;
                encoded_size(size)?;
                if size == 0 {
                    encoded_table(payload, 12, word(payload, 8)?, 4, cancel)?;
                }
            }
            b"stz2" => budget.samples_seen(payload, 8)?,
            b"trun" => {
                budget.samples_seen(payload, 4)?;
                let flags = word(payload, 0)?;
                if flags & 0x200 != 0 {
                    let stride = [0x100, 0x200, 0x400, 0x800]
                        .into_iter()
                        .filter(|flag| flags & flag != 0)
                        .count()
                        * 4;
                    let start = 8
                        + usize::from(flags & 1 != 0) * 4
                        + usize::from(flags & 4 != 0) * 4
                        + usize::from(flags & 0x100 != 0) * 4;
                    encoded_table(payload, start, word(payload, 4)?, stride, cancel)?;
                }
            }
            b"trex" => encoded_size(word(payload, 16)?)?,
            b"tfhd" => {
                let flags = word(payload, 0)?;
                if flags & 0x10 != 0 {
                    let at = 8
                        + usize::from(flags & 1 != 0) * 8
                        + usize::from(flags & 2 != 0) * 4
                        + usize::from(flags & 8 != 0) * 4;
                    encoded_size(word(payload, at)?)?;
                }
            }
            _ => {}
        }
        at += h.size;
    }
    Ok(())
}

pub(crate) fn preflight(path: &Path, cancel: &AtomicBool) -> Result<(), TimelineError> {
    let file = std::fs::File::open(path).map_err(|_| TimelineError::Unreadable)?;
    check_cancel(cancel)?;
    let mut head = [0_u8; 12];
    if file.len() < head.len() as u64 {
        return Ok(());
    }
    file.read_at(0, &mut head)
        .map_err(|_| TimelineError::Unreadable)?;
    // Exactly the same detector as FilmCraft's MP4 reader opener. Other formats remain handled
    // by their own readers; every source the MP4 opener accepts is preflighted first.
    if filmcraft_codecs::mp4::sniff(&head) {
        reader(&file, cancel)
    } else {
        Ok(())
    }
}

pub(super) fn reader(source: &impl ByteSource, cancel: &AtomicBool) -> Result<(), TimelineError> {
    check_cancel(cancel)?;
    let len = source.len();
    let mut budget = Budget::default();
    let (mut at, mut index_bytes) = (0_u64, 0_u64);
    while at < len {
        budget.box_seen(cancel)?;
        let h = header(source, at, len)?;
        if matches!(&h.kind, b"moov" | b"moof") {
            index_bytes = index_bytes
                .checked_add(h.size)
                .filter(|&n| n <= MAX_INDEX_BYTES)
                .ok_or(TimelineError::Unsupported)?;
            let size =
                usize::try_from(h.size - h.header).map_err(|_| TimelineError::Unsupported)?;
            let mut bytes = vec![0_u8; size];
            source
                .read_at(at + h.header, &mut bytes)
                .map_err(|_| TimelineError::Unreadable)?;
            children(&bytes, 1, &mut budget, cancel)?;
        }
        at += h.size;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend(kind);
        bytes.extend(payload);
        bytes
    }
    fn index(table: Vec<u8>) -> Vec<u8> {
        [b"stbl", b"minf", b"mdia", b"trak", b"moov"]
            .into_iter()
            .fold(table, |payload, kind| boxed(kind, &payload))
    }

    #[test]
    fn expanding_and_aggregate_sample_counts_fail_before_demux_allocation() {
        for kind in [b"stsz", b"stz2"] {
            let mut payload = vec![0; 8];
            payload.extend(1_000_001_u32.to_be_bytes());
            assert_eq!(
                reader(&index(boxed(kind, &payload)), &AtomicBool::new(false)),
                Err(TimelineError::Unsupported)
            );
        }
        let mut payload = vec![0; 4];
        payload.extend(1_000_001_u32.to_be_bytes());
        let bytes = boxed(b"moof", &boxed(b"traf", &boxed(b"trun", &payload)));
        assert_eq!(
            reader(&bytes, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
        let mut payload = vec![0; 8];
        payload[4..8].copy_from_slice(&1_u32.to_be_bytes());
        payload.extend(600_000_u32.to_be_bytes());
        let mut bytes = index(boxed(b"stsz", &payload));
        bytes.extend(index(boxed(b"stsz", &payload)));
        assert_eq!(
            reader(&bytes, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
    }

    #[test]
    fn aggregate_index_box_sizes_headers_depth_and_box_counts_are_bounded() {
        struct OversizedIndex;
        impl ByteSource for OversizedIndex {
            fn len(&self) -> u64 {
                MAX_INDEX_BYTES + 1
            }
            fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
                assert_eq!(
                    (offset, buffer.len()),
                    (0, 8),
                    "body must not be read or allocated"
                );
                buffer[..4].copy_from_slice(&((MAX_INDEX_BYTES + 1) as u32).to_be_bytes());
                buffer[4..8].copy_from_slice(b"moov");
                Ok(())
            }
        }
        assert_eq!(
            reader(&OversizedIndex, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
        let payload = boxed(b"free", &vec![0; 9 * 1024 * 1024]);
        let mut bytes = boxed(b"moov", &payload);
        bytes.extend(boxed(b"moof", &payload));
        assert_eq!(
            reader(&bytes, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
        for size in [0_u64, 7, u64::MAX] {
            let mut bytes = vec![0, 0, 0, 1];
            bytes.extend(b"moov");
            bytes.extend(size.to_be_bytes());
            assert!(reader(&bytes, &AtomicBool::new(false)).is_err());
        }
        let mut deep = Vec::new();
        for _ in 0..10 {
            deep = boxed(b"moov", &deep);
        }
        assert_eq!(
            reader(&deep, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
        let bytes = boxed(b"free", &[]).repeat(MAX_BOXES as usize + 1);
        assert_eq!(
            reader(&bytes, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
        assert_eq!(
            reader(&boxed(b"moov", &[]), &AtomicBool::new(true)),
            Err(TimelineError::Cancelled)
        );
    }

    #[test]
    fn oversized_explicit_and_default_encoded_sizes_are_rejected() {
        for constant in [false, true] {
            let mut payload = vec![0; 4];
            payload.extend(if constant { u32::MAX } else { 0 }.to_be_bytes());
            payload.extend(1_u32.to_be_bytes());
            if !constant {
                payload.extend(u32::MAX.to_be_bytes());
            }
            let bytes = index(boxed(b"stsz", &payload));
            assert_eq!(
                reader(&bytes, &AtomicBool::new(false)),
                Err(TimelineError::Unsupported)
            );
        }
        for (kind, flags) in [(b"tfhd", 0x10_u32), (b"trun", 0x200)] {
            let mut payload = flags.to_be_bytes().to_vec();
            payload.extend(1_u32.to_be_bytes());
            payload.extend(u32::MAX.to_be_bytes());
            let bytes = boxed(b"moof", &boxed(b"traf", &boxed(kind, &payload)));
            assert_eq!(
                reader(&bytes, &AtomicBool::new(false)),
                Err(TimelineError::Unsupported)
            );
        }
        let mut payload = vec![0; 16];
        payload.extend(u32::MAX.to_be_bytes());
        payload.extend([0; 4]);
        let bytes = boxed(b"moov", &boxed(b"mvex", &boxed(b"trex", &payload)));
        assert_eq!(
            reader(&bytes, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
        let mut payload = vec![0; 8];
        payload.extend(1_000_001_u32.to_be_bytes());
        let mut bytes = boxed(b"uuid", &[]);
        bytes.extend(index(boxed(b"stsz", &payload)));
        assert_eq!(
            reader(&bytes, &AtomicBool::new(false)),
            Err(TimelineError::Unsupported)
        );
    }
}
