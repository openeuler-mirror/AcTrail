//! Acceptance of workload observations across the wire codec and durable store.
use std::sync::Arc;
use std::time::Duration;

use sandbox_evidence_sqlite::{
    CURRENT_SCHEMA_VERSION, SandboxEvidenceSqliteConfig, SandboxEvidenceSqliteStore,
    SandboxEvidenceSynchronous,
};
use sandbox_evidence_store::{
    NoInterestEvidenceBatch, SandboxEvidenceAdmission, SandboxEvidenceSource,
    sandbox_observation::{
        GuestBootId, GuestPressureSnapshot, NormalizedContainerId, Observation, ObservationBatch,
        ProcessMarker, PsiAverages, SandboxContainerRuntime, WorkloadCgroupCounters,
        WorkloadCgroupId, WorkloadCgroupResourceSnapshot,
    },
};
use sandbox_vsock_contract::ObservationBatchCodec;

fn persist_and_reopen(observations: Vec<Observation>) {
    let codec = ObservationBatchCodec;
    let batch = ObservationBatch::new(17, observations.clone());
    let bytes = codec.encode(&batch).unwrap();
    let decoded = codec.decode(&bytes).unwrap();
    assert_eq!(decoded.observations, observations);
    let directory = tempfile::tempdir().unwrap();
    let config = SandboxEvidenceSqliteConfig {
        path: directory.path().join("evidence.sqlite"),
        schema_version: CURRENT_SCHEMA_VERSION,
        create_parent_directory: true,
        busy_timeout: Duration::from_secs(2),
        writer_queue_capacity: 16,
        batch_max_observations: 64,
        transaction_max_batches: 8,
        flush_interval: Duration::from_millis(5),
        retention_max_observations: 1024,
        capacity_max_bytes: 16 * 1024 * 1024,
        synchronous: SandboxEvidenceSynchronous::Full,
        wal_autocheckpoint_pages: 100,
        shutdown_drain_timeout: Duration::from_secs(5),
        writer_thread_stack_bytes: 256 * 1024,
        read_limit_max: 1024,
    };
    let source = SandboxEvidenceSource::new(3, 7).unwrap();
    let count = observations.len() as u32;
    let indices: Arc<[u32]> = (0..count).collect::<Vec<_>>().into();
    let evidence = NoInterestEvidenceBatch::new(
        source,
        decoded.sequence,
        11,
        decoded.observations.into(),
        indices,
    )
    .unwrap();
    let mut store = SandboxEvidenceSqliteStore::start(config.clone()).unwrap();
    assert_eq!(
        store.write_port().try_append_batch(evidence),
        SandboxEvidenceAdmission::Accepted {
            observation_count: count
        }
    );
    store.shutdown().unwrap();
    drop(store);
    let mut reopened = SandboxEvidenceSqliteStore::start(config).unwrap();
    let mut records = reopened.read_port().recent(64).unwrap();
    records.sort_by_key(|record| record.observation_index);
    assert_eq!(records.len(), observations.len());
    for (record, expected) in records.iter().zip(observations) {
        assert_eq!(record.source, source);
        assert_eq!(record.batch_sequence, 17);
        assert_eq!(record.route_generation, 11);
        assert_eq!(record.observation, expected);
    }
    reopened.shutdown().unwrap();
}

#[test]
fn workloads_and_pressure_survive_wire_encoding_storage_and_reopen() {
    let mut observations: Vec<_> = [1, 2]
        .into_iter()
        .map(|n| {
            Observation::WorkloadCgroup(WorkloadCgroupResourceSnapshot {
                guest_boot_id: GuestBootId::new([9; 16]),
                sampled_at_ms: 42,
                workload_id: WorkloadCgroupId::from_bytes([n; 32]),
                representative_root: ProcessMarker {
                    pid: 100 + u32::from(n),
                    start_time_ticks: 10,
                    executable_name: [0; 16],
                },
                monitored_root_count: 2,
                runtime: SandboxContainerRuntime::Containerd,
                container_id: Some(
                    NormalizedContainerId::from_lower_hex(&format!("{:064x}", n)).unwrap(),
                ),
                counters: WorkloadCgroupCounters {
                    memory_current_bytes: u64::from(n) * 1024,
                    memory_peak_bytes: Some(u64::MAX - u64::from(n)),
                    cpu_usage_usec: Some(123),
                    ..WorkloadCgroupCounters::default()
                },
            })
        })
        .collect();
    observations.insert(
        1,
        Observation::GuestPressure(GuestPressureSnapshot {
            guest_boot_id: GuestBootId::new([9; 16]),
            sampled_at_ms: 42,
            memory_some: PsiAverages {
                avg10_millipercent: 30_000,
                ..PsiAverages::default()
            },
            memory_full: PsiAverages::default(),
        }),
    );
    persist_and_reopen(observations);
}

#[test]
#[ignore = "run inside a guest with an idle workload in /default/<64hex> or /k8s.io/<64hex>"]
fn live_guest_counters_survive_wire_and_evidence_storage() {
    let name = std::env::var("ACTRAIL_TEST_GUEST_ROOT_COMM")
        .expect("set ACTRAIL_TEST_GUEST_ROOT_COMM to the idle workload's process comm");
    assert!(!name.is_empty() && name.len() < 16);
    let mut comm = [0; 16];
    comm[..name.len()].copy_from_slice(name.as_bytes());
    let collector =
        sandbox_linux_collector::WorkloadCgroupCollector::start("/proc".into(), vec![comm])
            .unwrap();
    let snapshots = collector.sample().unwrap();
    assert!(
        !snapshots.is_empty(),
        "no supported guest workload was discovered"
    );
    let mut identities = std::collections::BTreeSet::new();
    for snapshot in &snapshots {
        assert!(
            identities.insert(snapshot.workload_id),
            "duplicate workload sample"
        );
        let membership =
            std::fs::read_to_string(format!("/proc/{}/cgroup", snapshot.representative_root.pid))
                .unwrap();
        let relative = membership
            .lines()
            .find_map(|line| line.strip_prefix("0::/"))
            .unwrap();
        let boundary = relative.split('/').take(2).collect::<Vec<_>>().join("/");
        let direct: u64 =
            std::fs::read_to_string(format!("/sys/fs/cgroup/{boundary}/memory.current"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
        assert!(
            snapshot.counters.memory_current_bytes.abs_diff(direct) <= 4 * 1024 * 1024,
            "idle guest workload changed by more than 4 MiB between reads"
        );
    }
    persist_and_reopen(
        snapshots
            .into_iter()
            .map(Observation::WorkloadCgroup)
            .collect(),
    );
}
