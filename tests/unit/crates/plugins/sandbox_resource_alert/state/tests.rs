// Tests for the corresponding production module.
use super::*;

fn source() -> SandboxSource {
    SandboxSource::new(1, 1).expect("test source identifiers are non-zero")
}

fn boot_id(value: u8) -> GuestBootId {
    GuestBootId::new([value; 16])
}

fn cpu(total_ticks: u64, idle_ticks: u64) -> CpuSnapshot {
    CpuSnapshot {
        total_ticks,
        idle_ticks,
        logical_cpu_count: 2,
    }
}

fn state_table() -> SourceStateTable {
    let mut states = SourceStateTable::new(1);
    states
        .begin_batch(source())
        .expect("test source state should be created");
    states
}

fn usage_between(previous: CpuSnapshot, current: CpuSnapshot) -> Option<u16> {
    let mut states = state_table();
    assert_eq!(
        states.update_cpu_sample(source(), boot_id(1), previous),
        Ok(None)
    );
    states
        .update_cpu_sample(source(), boot_id(1), current)
        .expect("test CPU sample should update")
}

#[test]
fn cpu_usage_requires_two_samples_from_the_same_boot() {
    let mut states = state_table();

    assert_eq!(
        states.update_cpu_sample(source(), boot_id(1), cpu(100, 25)),
        Ok(None)
    );
    assert_eq!(
        states.update_cpu_sample(source(), boot_id(1), cpu(200, 50)),
        Ok(Some(7_500))
    );

    assert_eq!(
        states.update_cpu_sample(source(), boot_id(2), cpu(10, 5)),
        Ok(None)
    );
    assert_eq!(
        states.update_cpu_sample(source(), boot_id(2), cpu(30, 10)),
        Ok(Some(7_500))
    );
}

#[test]
fn cpu_usage_is_reported_in_basis_points() {
    assert_eq!(usage_between(cpu(1_000, 400), cpu(1_120, 430)), Some(7_500));
    assert_eq!(usage_between(cpu(10, 5), cpu(13, 6)), Some(6_666));
}

#[test]
fn invalid_cpu_deltas_are_ignored() {
    assert_eq!(usage_between(cpu(100, 20), cpu(100, 20)), None);
    assert_eq!(usage_between(cpu(100, 20), cpu(99, 21)), None);
    assert_eq!(usage_between(cpu(100, 20), cpu(110, 19)), None);
    assert_eq!(usage_between(cpu(100, 20), cpu(105, 30)), None);
}

#[test]
fn cpu_risk_is_emitted_once_until_recovery() {
    let mut states = state_table();

    assert_eq!(states.update_cpu_risk(source(), false), Ok(false));
    assert_eq!(states.update_cpu_risk(source(), true), Ok(true));
    assert_eq!(states.update_cpu_risk(source(), true), Ok(false));
    assert_eq!(states.update_cpu_risk(source(), false), Ok(false));
    assert_eq!(states.update_cpu_risk(source(), true), Ok(true));
}
