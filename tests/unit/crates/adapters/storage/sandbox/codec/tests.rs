// Tests for the corresponding production module.
use super::*;
use sandbox_evidence_store::sandbox_observation::GuestBootId;

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn snapshot() -> WorkloadCgroupResourceSnapshot {
    WorkloadCgroupResourceSnapshot {
        guest_boot_id: GuestBootId::new([0x11; 16]),
        sampled_at_ms: 42,
        workload_id: WorkloadCgroupId::from_bytes([0x22; 32]),
        representative_root: ProcessMarker {
            pid: 7,
            start_time_ticks: 8,
            executable_name: [0; 16],
        },
        monitored_root_count: 1,
        runtime: SandboxContainerRuntime::Containerd,
        container_id: Some(NormalizedContainerId::from_lower_hex(ID).unwrap()),
        counters: WorkloadCgroupCounters {
            memory_current_bytes: 100,
            memory_peak_bytes: Some(200),
            cpu_usage_usec: Some(300),
            ..WorkloadCgroupCounters::default()
        },
    }
}

#[test]
fn workload_cgroup_kind_round_trips_through_storage_codec() {
    let (kind, payload) = ObservationCodec::encode(&Observation::WorkloadCgroup(snapshot()));
    assert_eq!(kind, WORKLOAD_CGROUP_KIND);
    assert_eq!(payload.len(), 320);
    let decoded = ObservationCodec::decode(kind, &payload).unwrap();
    assert_eq!(decoded, Observation::WorkloadCgroup(snapshot()));
}

#[test]
fn workload_cgroup_storage_decode_rejects_unknown_kind() {
    let (_, payload) = ObservationCodec::encode(&Observation::WorkloadCgroup(snapshot()));
    assert!(ObservationCodec::decode(9, &payload).is_err());
}
