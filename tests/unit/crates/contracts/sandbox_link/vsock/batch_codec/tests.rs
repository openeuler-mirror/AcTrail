// Tests for the corresponding production module.
use super::*;
use sandbox_observation::GuestBootId;

const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn full_snapshot() -> WorkloadCgroupResourceSnapshot {
    WorkloadCgroupResourceSnapshot {
        guest_boot_id: GuestBootId::new([0x11; 16]),
        sampled_at_ms: 0x0102_0304_0506_0708,
        workload_id: WorkloadCgroupId::from_bytes([0x22; 32]),
        representative_root: ProcessMarker {
            pid: 1234,
            start_time_ticks: 5678,
            executable_name: *b"root-process!!!!",
        },
        monitored_root_count: 3,
        runtime: SandboxContainerRuntime::Docker,
        container_id: Some(NormalizedContainerId::from_lower_hex(ID).unwrap()),
        counters: WorkloadCgroupCounters {
            memory_current_bytes: 1,
            memory_peak_bytes: Some(2),
            memory_anon_bytes: Some(3),
            memory_file_bytes: Some(4),
            memory_swap_current_bytes: Some(5),
            memory_low: Some(6),
            memory_high: Some(7),
            memory_max: Some(8),
            memory_oom: Some(9),
            memory_oom_kill: Some(10),
            memory_oom_group_kill: Some(11),
            cpu_usage_usec: Some(12),
            cpu_user_usec: Some(13),
            cpu_system_usec: Some(14),
            cpu_nr_throttled: Some(15),
            cpu_throttled_usec: Some(16),
            io_read_bytes: Some(17),
            io_write_bytes: Some(18),
            pids_current: Some(19),
            pids_peak: Some(20),
        },
    }
}

fn minimal_snapshot() -> WorkloadCgroupResourceSnapshot {
    WorkloadCgroupResourceSnapshot {
        guest_boot_id: GuestBootId::new([0x33; 16]),
        sampled_at_ms: 7,
        workload_id: WorkloadCgroupId::from_bytes([0x44; 32]),
        representative_root: ProcessMarker {
            pid: 99,
            start_time_ticks: 100,
            executable_name: [0; 16],
        },
        monitored_root_count: 1,
        runtime: SandboxContainerRuntime::Unknown,
        container_id: None,
        counters: WorkloadCgroupCounters {
            memory_current_bytes: 42,
            ..WorkloadCgroupCounters::default()
        },
    }
}

fn codec() -> ObservationBatchCodec {
    ObservationBatchCodec
}

#[test]
fn workload_cgroup_body_is_exactly_320_bytes_and_round_trips() {
    let mut body = Vec::new();
    codec().encode_workload_cgroup(&mut body, &full_snapshot());
    assert_eq!(body.len(), 320);
    assert_eq!(body[0], 1); // payload version
    assert_eq!(body[1], SandboxContainerRuntime::Docker.code());
    assert_eq!(body[2], NormalizedContainerId::HEX_LENGTH as u8);
    assert_eq!(body[3], 0); // reserved

    let decoded = codec().decode_workload_cgroup(&body).unwrap();
    assert_eq!(decoded, full_snapshot());
}

#[test]
fn workload_cgroup_round_trips_absent_fields_and_container() {
    let mut body = Vec::new();
    codec().encode_workload_cgroup(&mut body, &minimal_snapshot());
    assert_eq!(body.len(), 320);
    assert_eq!(body[2], 0); // absent container ID
    let decoded = codec().decode_workload_cgroup(&body).unwrap();
    assert_eq!(decoded, minimal_snapshot());
    assert!(decoded.counters.memory_peak_bytes.is_none());
    assert!(decoded.counters.cpu_usage_usec.is_none());
    assert_eq!(decoded.container_id, None);
}

#[test]
fn workload_cgroup_round_trips_through_batch_codec() {
    let batch = ObservationBatch::new(
        9,
        vec![
            Observation::WorkloadCgroup(full_snapshot()),
            Observation::WorkloadCgroup(minimal_snapshot()),
        ],
    );
    let encoded = codec().encode(&batch).unwrap();
    let decoded = codec().decode(&encoded).unwrap();
    assert_eq!(decoded.sequence, 9);
    assert_eq!(decoded.observations.len(), 2);
    assert_eq!(
        decoded.observations[0],
        Observation::WorkloadCgroup(full_snapshot())
    );
    assert_eq!(
        decoded.observations[1],
        Observation::WorkloadCgroup(minimal_snapshot())
    );
}

#[test]
fn workload_cgroup_rejects_invalid_version_runtime_and_zero_roots() {
    let mut body = Vec::new();
    codec().encode_workload_cgroup(&mut body, &minimal_snapshot());

    let mut bad_version = body.clone();
    bad_version[0] = 2;
    assert!(codec().decode_workload_cgroup(&bad_version).is_err());

    let mut bad_runtime = body.clone();
    bad_runtime[1] = 6;
    assert!(codec().decode_workload_cgroup(&bad_runtime).is_err());

    let mut bad_reserved = body.clone();
    bad_reserved[3] = 1;
    assert!(codec().decode_workload_cgroup(&bad_reserved).is_err());

    let mut zero_roots = body.clone();
    zero_roots[68..72].copy_from_slice(&0_u32.to_be_bytes());
    assert!(codec().decode_workload_cgroup(&zero_roots).is_err());

    let mut high_presence = body.clone();
    high_presence[4..8].copy_from_slice(&0x8000_0000_u32.to_be_bytes());
    assert!(codec().decode_workload_cgroup(&high_presence).is_err());
}

#[test]
fn workload_cgroup_rejects_truncated_and_trailing_bytes() {
    let mut body = Vec::new();
    codec().encode_workload_cgroup(&mut body, &minimal_snapshot());

    assert!(codec().decode_workload_cgroup(&body[..319]).is_err());

    let mut trailing = body.clone();
    trailing.push(0);
    // decode_workload_cgroup does not itself check trailing bytes; the batch
    // codec does. Verify batch-level trailing rejection instead.
    let batch = ObservationBatch::new(1, vec![Observation::WorkloadCgroup(minimal_snapshot())]);
    let mut encoded = codec().encode(&batch).unwrap();
    encoded.push(0);
    assert!(codec().decode(&encoded).is_err());
}
