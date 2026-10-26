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
        self.expected_start_time_ticks(host)
            .is_some_and(|expected| current.start_time_ticks != expected)
    }

    /// Return this identity's start time in the observer's procfs clock.
    /// Unknown generations or an unverifiable observer clock return `None`.
    /// The observer must keep the same time namespace for its lifetime.
    pub fn expected_start_time_ticks(&self, host: &HostProcessCoordinates) -> Option<u64> {
        if host.start_time_ticks != 0 {
            Some(host.start_time_ticks)
        } else {
            host.start_boottime_ns
                .filter(|value| *value != 0)
                .and_then(|ns| ProcfsStartClock::get()?.start_ticks(ns))
        }
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
        // timens_offsets is exposed at /proc/<tid>, but not under
        // /proc/<pid>/task/<tid> (the target of /proc/thread-self).
        let observer = std::path::PathBuf::from(format!("/proc/{}", unsafe { libc::gettid() }));
        let current = std::fs::read_link(observer.join("ns/time")).ok()?;
        let children = std::fs::read_link(observer.join("ns/time_for_children")).ok()?;
        if current != children {
            return None;
        }
        let offsets = std::fs::read_to_string(observer.join("timens_offsets")).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_time_conversion_uses_observer_offset_and_tick_rate() {
        for (offset, expected) in [
            (0, 10_000),
            (60_000_000_000, 16_000),
            (-60_000_000_000, 4_000),
        ] {
            let clock = ProcfsStartClock {
                nanos_per_tick: 10_000_000,
                boottime_offset_ns: offset,
            };
            assert_eq!(clock.start_ticks(100_000_000_000), Some(expected));
        }
        for (hz, expected) in [(100, 123), (250, 308), (1000, 1234)] {
            let clock = ProcfsStartClock {
                nanos_per_tick: 1_000_000_000 / hz,
                boottime_offset_ns: 0,
            };
            assert_eq!(clock.start_ticks(1_234_567_890), Some(expected));
        }
        let clock = ProcfsStartClock {
            nanos_per_tick: 10_000_000,
            boottime_offset_ns: 5_000_000,
        };
        // Apply sub-tick offsets before truncating to procfs ticks.
        assert_eq!(clock.start_ticks(1_235_000_000), Some(124));
    }

    #[test]
    fn boot_time_conversion_rejects_out_of_range_observer_time() {
        let mut clock = ProcfsStartClock {
            nanos_per_tick: 10_000_000,
            boottime_offset_ns: 0,
        };
        assert_eq!(clock.start_ticks(u64::MAX), Some(1_844_674_407_370));
        clock.boottime_offset_ns = 1;
        assert_eq!(clock.start_ticks(u64::MAX), None);
        clock.boottime_offset_ns = -2;
        assert_eq!(clock.start_ticks(1), None);
    }

    #[test]
    fn observer_clock_is_readable_from_a_worker_thread() {
        std::thread::spawn(|| {
            let clock = ProcfsStartClock::read().expect("observer clock should be readable");
            assert!(clock.nanos_per_tick > 0);
        })
        .join()
        .unwrap();
    }
}
