// Tests for the corresponding production module.
use super::*;

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn normalized_container_id_requires_lowercase_hex_and_round_trips() {
    let id = NormalizedContainerId::from_lower_hex(ID).unwrap();
    assert_eq!(id.to_lower_hex(), ID);
    assert_eq!(NormalizedContainerId::from_bytes(*id.as_bytes()), id);
    assert!(NormalizedContainerId::from_lower_hex(&ID[..63]).is_err());
    assert!(NormalizedContainerId::from_lower_hex(&ID.to_uppercase()).is_err());
}

#[test]
fn runtime_codes_are_stable_and_bounded() {
    for runtime in [
        SandboxContainerRuntime::Unknown,
        SandboxContainerRuntime::Docker,
        SandboxContainerRuntime::Containerd,
        SandboxContainerRuntime::KubernetesUnknownCri,
        SandboxContainerRuntime::Podman,
        SandboxContainerRuntime::Crio,
    ] {
        assert_eq!(
            SandboxContainerRuntime::from_code(runtime.code()),
            Some(runtime)
        );
    }
    assert_eq!(SandboxContainerRuntime::from_code(6), None);
    assert_eq!(SandboxContainerRuntime::from_code(255), None);
}

#[test]
fn workload_id_rejects_non_canonical_paths() {
    let boot = GuestBootId::new([0; 16]);
    for path in [
        &b""[..],
        &b"relative"[..],
        &b"/docker//abc"[..],
        &b"/docker/./abc"[..],
        &b"/docker/../abc"[..],
        &b"/docker/"[..],
    ] {
        assert_eq!(
            WorkloadCgroupId::compute(&boot, 1, 2, 3, path),
            Err(WorkloadCgroupIdError::PathNotCanonical),
            "path {path:?} should be rejected"
        );
    }
}

#[test]
fn workload_id_is_deterministic_and_sensitive_to_inputs() {
    let boot = GuestBootId::new([7; 16]);
    let a = WorkloadCgroupId::compute(&boot, 1, 2, 3, b"/docker/abc").unwrap();
    let b = WorkloadCgroupId::compute(&boot, 1, 2, 3, b"/docker/abc").unwrap();
    assert_eq!(a, b);
    let other_path = WorkloadCgroupId::compute(&boot, 1, 2, 3, b"/docker/abd").unwrap();
    assert_ne!(a, other_path);
    let other_boot =
        WorkloadCgroupId::compute(&GuestBootId::new([8; 16]), 1, 2, 3, b"/docker/abc").unwrap();
    assert_ne!(a, other_boot);
}

#[test]
fn snapshot_validation_requires_positive_roots_and_real_marker() {
    let boot = GuestBootId::new([0; 16]);
    let workload_id = WorkloadCgroupId::from_bytes([0; 32]);
    let marker = ProcessMarker {
        pid: 42,
        start_time_ticks: 9,
        executable_name: [0; 16],
    };
    let snapshot = WorkloadCgroupResourceSnapshot {
        guest_boot_id: boot,
        sampled_at_ms: 0,
        workload_id,
        representative_root: marker,
        monitored_root_count: 1,
        runtime: SandboxContainerRuntime::Docker,
        container_id: None,
        counters: WorkloadCgroupCounters::default(),
    };
    assert_eq!(snapshot.validate(), Ok(()));

    let mut zero_count = snapshot;
    zero_count.monitored_root_count = 0;
    assert!(zero_count.validate().is_err());

    let mut zero_marker = snapshot;
    zero_marker.representative_root = ProcessMarker {
        pid: 0,
        start_time_ticks: 0,
        executable_name: [0; 16],
    };
    assert!(zero_marker.validate().is_err());
}
