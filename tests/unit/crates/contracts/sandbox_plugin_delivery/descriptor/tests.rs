// Tests for the corresponding production module.
use super::*;
use sandbox_observation::{GuestBootId, GuestPressureSnapshot, PsiAverages};

#[test]
fn guest_pressure_kind_index_and_classification() {
    let observation = Observation::GuestPressure(GuestPressureSnapshot {
        guest_boot_id: GuestBootId::new([0; 16]),
        sampled_at_ms: 1,
        memory_some: PsiAverages::default(),
        memory_full: PsiAverages::default(),
    });
    assert_eq!(
        SandboxObservationKind::of(&observation),
        SandboxObservationKind::GuestPressure
    );
    assert_eq!(SandboxObservationKind::GuestPressure.index(), 3);
    assert_eq!(SandboxObservationKind::COUNT, 5);
}
