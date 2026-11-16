// Tests for the corresponding production module.
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
