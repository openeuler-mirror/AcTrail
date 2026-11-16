// Tests for the corresponding production module.
use super::*;
use model_core::container::{ContainerRuntime, NormalizedContainerId};
use model_core::ids::{OtelTraceId, ProfileName, TraceId, TraceName};
use model_core::process::{HostProcessCoordinates, ProcessMembership};
use model_core::trace::{TraceAlertToken, TraceRecord};
use resource_profiling::model::cgroup_v2_binding::{CgroupV2Binding, HostBootId};

#[test]
fn binding_failure_rolls_back_trace_processes_and_memberships_then_retry_commits() {
    let temp = tempfile::tempdir().unwrap();
    let mut storage = storage_factory::open_storage_backend(
        &storage_contracts::StorageConfig::sqlite_path(temp.path().join("admission.sqlite")),
        storage_core::StorageOpenMode::ReadWrite,
    )
    .unwrap();
    let id = TraceId::new(1);
    let process = ProcessIdentity::new(1);
    let trace = TraceRecord::new(
        id,
        OtelTraceId::from_bytes([1; 16]).unwrap(),
        TraceAlertToken::new([0; 32]),
        process,
        TraceName::new("atomic"),
        ProfileName::new("test"),
        SystemTime::now(),
    );
    let record = ProcessRecord::new(
        process,
        ProcessObservation::host(HostProcessCoordinates::new(42, 900)),
    );
    let member = ProcessMembership::root(id, process, SystemTime::now());
    let mut binding = CgroupV2Binding::active(
        id,
        ContainerRuntime::Unknown,
        NormalizedContainerId::from_lower_hex(&"ab".repeat(32)).unwrap(),
        format!("docker/{}", "ab".repeat(32)),
        1,
        2,
        HostBootId::from_bytes([1; 16]),
        SystemTime::now(),
    );
    assert!(
        persist_admitted_trace(
            storage.as_mut(),
            trace.clone(),
            vec![member.clone()],
            vec![record.clone()],
            Some(binding.clone()),
            None
        )
        .is_err()
    );
    assert!(storage.get_trace(id).unwrap().is_none());
    assert!(storage.get_process_record(process).unwrap().is_none());
    assert!(storage.trace_memberships(id).unwrap().is_empty());
    assert!(storage.get_cgroup_v2_binding(id).unwrap().is_none());
    binding.runtime = ContainerRuntime::Docker;
    persist_admitted_trace(
        storage.as_mut(),
        trace,
        vec![member],
        vec![record],
        Some(binding),
        None,
    )
    .unwrap();
    assert!(storage.get_trace(id).unwrap().is_some());
    assert!(storage.get_cgroup_v2_binding(id).unwrap().is_some());
    storage.discard_orphan_cgroup_v2_binding(id).unwrap();
    assert!(storage.get_cgroup_v2_binding(id).unwrap().is_some());
}
