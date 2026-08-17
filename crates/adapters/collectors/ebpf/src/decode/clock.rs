//! Anchoring of kernel monotonic timestamps to wall-clock `SystemTime`.
//!
//! eBPF events carry `observed_ktime_ns` stamped by `bpf_ktime_get_ns()`,
//! which reads the kernel's `CLOCK_MONOTONIC`. Using `SystemTime::now()` at
//! decode time stamps events by *consumption* order instead of *capture*
//! order, which breaks request/response correlation whenever the transport
//! delivers events out of causal order — ring buffers are drained per-CPU, so
//! an inbound chunk can be handed to userspace before the outbound request
//! that caused it. Deriving `observed_at` from the kernel timestamp keeps the
//! ordering keys consistent with capture order regardless of delivery order.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// Anchor mapping the kernel monotonic clock to wall-clock time.
#[derive(Clone, Copy, Debug)]
struct KernelClock {
    wall: SystemTime,
    monotonic_ns: u64,
}

impl KernelClock {
    fn capture() -> Self {
        let (wall, monotonic_ns) = monotonic_ns_with_wall();
        Self { wall, monotonic_ns }
    }

    fn wall_from_ktime(&self, ktime_ns: u64) -> SystemTime {
        let offset = Duration::from_nanos(ktime_ns.saturating_sub(self.monotonic_ns));
        self.wall + offset
    }
}

fn monotonic_ns_with_wall() -> (SystemTime, u64) {
    let mut ts = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: `ts` points to writable memory of the correct type; the kernel
    // writes the current CLOCK_MONOTONIC value on success.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, ts.as_mut_ptr()) };
    let monotonic_ns = if rc == 0 {
        // SAFETY: clock_gettime returned success, so the timespec is initialized.
        let ts = unsafe { ts.assume_init() };
        (ts.tv_sec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(u64::try_from(ts.tv_nsec).unwrap_or(0))
    } else {
        0
    };
    // Read the wall clock after the monotonic read so the anchor offset is a
    // lower bound on real elapsed time.
    (SystemTime::now(), monotonic_ns)
}

static GLOBAL_CLOCK: OnceLock<KernelClock> = OnceLock::new();

/// Convert a kernel `bpf_ktime_get_ns()` timestamp into a wall-clock
/// `SystemTime`, anchored once per process at first use.
///
/// Timestamps earlier than the anchor (events captured before the daemon
/// started, or synthetic test values) clamp to the anchor wall time instead
/// of producing a time before the UNIX epoch.
pub(super) fn wall_from_ktime(ktime_ns: u64) -> SystemTime {
    GLOBAL_CLOCK
        .get_or_init(KernelClock::capture)
        .wall_from_ktime(ktime_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_ktime_clamps_to_anchor() {
        let clock = KernelClock::capture();
        let wall = clock.wall_from_ktime(0);
        assert!(wall >= SystemTime::UNIX_EPOCH);
    }

    #[test]
    fn ktime_order_is_preserved() {
        let clock = KernelClock::capture();
        let earlier = clock.wall_from_ktime(1_000);
        let later = clock.wall_from_ktime(2_000);
        assert!(earlier <= later);
    }
}
