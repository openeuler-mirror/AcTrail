use std::sync::OnceLock;

use model_core::process::HostProcessCoordinates;
use process_identity::IdentityLookupError;

use super::{ProcfsIdentityReader, read_stat};

impl ProcfsIdentityReader {
    /// Check an existing identity without resolving namespaces or enriching it.
    /// Unknown generations and inaccessible procfs entries remain observable.
    pub fn process_is_gone(&self, host: &HostProcessCoordinates) -> bool {
        let current = match read_stat(host.pid) {
            Ok(current) => current,
            Err(IdentityLookupError::NotFound { .. }) => return true,
            Err(_) => return false,
        };
        let expected_ticks = if host.start_time_ticks != 0 {
            Some(host.start_time_ticks)
        } else {
            host.start_boottime_ns
                .filter(|value| *value != 0)
                .and_then(|ns| ProcfsStartClock::get()?.start_ticks(ns))
        };
        expected_ticks.is_some_and(|expected| current.start_time_ticks != expected)
    }
}

struct ProcfsStartClock {
    nanos_per_tick: u64,
    boottime_offset_ns: i128,
}

impl ProcfsStartClock {
    fn get() -> Option<&'static Self> {
        // The daemon never changes its time namespace. Its offset is frozen
        // once populated, and USER_HZ is fixed. Cache only these environment
        // constants, never process state; failed reads can retry next time.
        // A future namespace-switching caller must not reuse this cache.
        static CLOCK: OnceLock<ProcfsStartClock> = OnceLock::new();
        if let Some(clock) = CLOCK.get() {
            return Some(clock);
        }
        let clock = Self::read()?;
        Some(CLOCK.get_or_init(|| clock))
    }

    fn read() -> Option<Self> {
        let ticks_per_second = crate::clock_ticks_per_second()?;
        if 1_000_000_000 % ticks_per_second != 0 {
            return None;
        }
        // timens_offsets describes time_for_children. It represents the
        // observer's current clock only when both namespaces are identical.
        let current = std::fs::read_link("/proc/thread-self/ns/time").ok()?;
        let children = std::fs::read_link("/proc/thread-self/ns/time_for_children").ok()?;
        if current != children {
            return None;
        }
        let offsets = std::fs::read_to_string("/proc/thread-self/timens_offsets").ok()?;
        for line in offsets.lines() {
            let mut fields = line.split_whitespace();
            if fields.next()? != "boottime" {
                continue;
            }
            let seconds = fields.next()?.parse::<i64>().ok()?;
            let nanos = fields.next()?.parse::<u32>().ok()?;
            if nanos >= 1_000_000_000 || fields.next().is_some() {
                return None;
            }
            return Some(Self {
                nanos_per_tick: 1_000_000_000 / ticks_per_second,
                boottime_offset_ns: i128::from(seconds) * 1_000_000_000 + i128::from(nanos),
            });
        }
        None
    }

    fn start_ticks(&self, start_boottime_ns: u64) -> Option<u64> {
        let observer_ns = i128::from(start_boottime_ns) + self.boottime_offset_ns;
        let observer_ns = u64::try_from(observer_ns).ok()?;
        Some(observer_ns / self.nanos_per_tick)
    }
}
