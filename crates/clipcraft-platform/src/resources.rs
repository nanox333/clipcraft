//! How much of the computer this program is using: processor share and memory. Read once a
//! second or so, so a runaway can be caught before it starves the rest of the machine.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// What the program is using right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelfUsage {
    /// Share of the whole processor, 0.0 to 1.0 (all cores busy is 1.0).
    pub cpu_share: f32,
    /// Memory held, in bytes.
    pub memory_bytes: u64,
}

/// Reads this program's own usage. The processor share is measured between two reads, so the
/// first read has none.
pub struct SelfMeter {
    system: System,
    pid: Pid,
    cores: f32,
    /// The processor time used and the moment it was read, at the last read.
    last: Option<(std::time::Instant, u64)>,
}

impl SelfMeter {
    /// `None` when the system will not say which process this is.
    pub fn new() -> Option<Self> {
        let pid = sysinfo::get_current_pid().ok()?;
        let cores = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .max(1);
        Some(Self {
            system: System::new(),
            pid,
            cores: cores as f32,
            last: None,
        })
    }

    /// Usage since the last read, or `None` for the first read or when it cannot be read.
    pub fn sample(&mut self) -> Option<SelfUsage> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        let process = self.system.process(self.pid)?;
        // Processor time used (milliseconds) over the time that passed, across all cores.
        let now = (std::time::Instant::now(), process.accumulated_cpu_time());
        let before = self.last.replace(now)?;
        let wall_ms = now.0.duration_since(before.0).as_secs_f32() * 1000.0;
        if wall_ms < 1.0 {
            return None;
        }
        let used_ms = now.1.saturating_sub(before.1) as f32;
        Some(SelfUsage {
            cpu_share: (used_ms / wall_ms / self.cores).clamp(0.0, 1.0),
            memory_bytes: process.memory(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meter_sees_its_own_memory_and_a_busy_loop() {
        let Some(mut meter) = SelfMeter::new() else {
            return;
        };
        assert!(
            meter.sample().is_none(),
            "the first read has nothing to compare with"
        );
        let end = std::time::Instant::now() + std::time::Duration::from_millis(1_200);
        let mut spin = 0_u64;
        while std::time::Instant::now() < end {
            spin = spin.wrapping_add(1);
        }
        std::hint::black_box(spin);
        let usage = meter.sample().expect("a second read");
        assert!(usage.memory_bytes > 1_000_000, "{usage:?}");
        // One core spinning is about 1/cores of the machine.
        let cores = std::thread::available_parallelism().map_or(1, usize::from) as f32;
        assert!(usage.cpu_share > 0.4 / cores, "{usage:?} on {cores} cores");
    }
}
