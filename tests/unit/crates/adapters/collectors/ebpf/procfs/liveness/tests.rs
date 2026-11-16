// Tests for the corresponding production module.
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
