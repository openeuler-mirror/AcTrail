//! Integration-oriented daemon service tests.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use config_core::capture_profile::CaptureProfile;
use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, CommandControlConfig,
    DEFAULT_ACTIVE_TRACE_MAX, DiagnosticLogLevel, EbpfCollectorConfig, EbpfEnabledMode,
    EnforcementConfig, FileObservationConfig, MemlockRlimit, NetworkControlConfig, OperatorConfig,
    PayloadConfig, ProcessSeccompConfig, ResourceMetricsConfig, RuntimeExportConfig,
    SeccompNotifyConfig, SemanticRetentionConfig, StorageRetentionConfig, TraceFinalizationConfig,
};
use config_core::trace_snapshot::CaptureProfileSnapshot;
use control_contract::command::{ControlCommand, ListTracesCommand, ProcessRef, TrackAddCommand};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::event::EventPayload;
use model_core::ids::{CollectorName, ProfileName, RequestId, TraceName};
use model_core::process::{NamespaceIdentity, ProcessIdentity};
use model_core::trace::{TraceHealth, TraceLifecycleState, TraceRecord};
use storage_core::TraceFilter;
use storage_factory::StorageConfig;
use trace_runtime::commands::TrackTraceRequest;
use trace_runtime::sensor_plan::{CollectorPlan, SensorPlan};
use uds_control_server::ControlService;

use crate::profiles::DaemonProfileRegistry;
use crate::service_host::DaemonServiceHost;

use super::{
    build_runtime_wiring, build_runtime_wiring_with_storage_retention,
    workload_diagnostics::WorkloadDiagnostics,
};

#[path = "test_cases/application_protocol.rs"]
mod application_protocol_tests;
#[path = "test_cases/lineage_projection.rs"]
mod lineage_projection_tests;
#[path = "test_cases/live_export.rs"]
mod live_export_tests;
#[path = "test_cases/tls_sync.rs"]
mod tls_sync_tests;

const RESOURCE_TEST_INTERVAL: Duration = Duration::from_millis(2);
const APPLICATION_PROTOCOL_COLLECTOR: &str = "application-protocol-analyzer";
const RESOURCE_METRICS_COLLECTOR: &str = "resource-sampler";
const TEST_HTTP_BUFFER_BYTES: u64 = 4096;
const TEST_HTTP2_MAX_FRAME_BYTES: u64 = 16384;
const TEST_HTTP2_PREVIEW_BYTES: u64 = 16;
static TEST_SOCKET_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn test_process_ref(pid: u32) -> ProcessRef {
    let namespace_path = format!("/proc/{pid}/ns/pid");
    let pid_namespace = std::fs::read_link(&namespace_path)
        .unwrap_or_else(|error| panic!("read {namespace_path}: {error}"));
    ProcessRef::new(
        pid,
        NamespaceIdentity::new(pid_namespace.display().to_string()),
    )
}

#[test]
fn attach_main_path_runs() {
    let storage_path =
        std::env::temp_dir().join(format!("actrail-test-{}.sqlite", std::process::id()));
    let mut profiles = DaemonProfileRegistry::new();
    profiles.insert_capture_profile(CaptureProfile::new(
        ProfileName::new("snapshot"),
        Vec::new(),
    ));
    let wiring = build_runtime_wiring(
        &test_storage_config(storage_path.clone()),
        profiles,
        ebpf_config(true),
        payload_config(false),
        DEFAULT_ACTIVE_TRACE_MAX,
        DiagnosticLogLevel::Info,
        SeccompNotifyConfig::disabled(),
        ProcessSeccompConfig::disabled(),
        AgentInvocationConfig::disabled(),
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        ApplicationProtocolConfig::disabled(),
        ResourceMetricsConfig::disabled(),
        TraceFinalizationConfig::default(),
        WorkloadDiagnostics::default(),
        RuntimeExportConfig::disabled(),
        EnforcementConfig::disabled(),
        CommandControlConfig::disabled(),
        NetworkControlConfig::disabled(),
    )
    .unwrap();
    let mut host = DaemonServiceHost::new(wiring);

    let reply = host
        .handle(ControlCommand::TrackAdd(TrackAddCommand {
            request_id: RequestId::new(1),
            root: test_process_ref(std::process::id()),
            display_name: TraceName::new("self"),
            profile_name: ProfileName::new("snapshot"),
            tags: BTreeSet::new(),
            launch_mode: false,
            initial_suppressed_fds: Vec::new(),
        }))
        .unwrap();
    let control_contract::reply::ControlReply::TrackAdded(reply) = reply else {
        panic!("unexpected reply");
    };

    let list = host
        .handle(ControlCommand::ListTraces(ListTracesCommand {
            request_id: RequestId::new(2),
            selector: None,
        }))
        .unwrap();
    let control_contract::reply::ControlReply::TraceList(items) = list else {
        panic!("unexpected list reply");
    };
    assert!(items.iter().any(|item| item.trace_id == reply.trace_id));
}

#[test]
fn direct_track_add_rejects_a_launch_only_profile() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-launch-profile-admission-test-{}.sqlite",
        std::process::id()
    ));
    let mut profiles = DaemonProfileRegistry::new();
    profiles.insert_launch_profile(CaptureProfile::new(
        ProfileName::new("snapshot-ebpf-off-notify-off"),
        Vec::new(),
    ));
    let wiring = build_runtime_wiring(
        &test_storage_config(storage_path),
        profiles,
        ebpf_config(false),
        payload_config(false),
        DEFAULT_ACTIVE_TRACE_MAX,
        DiagnosticLogLevel::Info,
        SeccompNotifyConfig::disabled(),
        ProcessSeccompConfig::disabled(),
        AgentInvocationConfig::disabled(),
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        ApplicationProtocolConfig::disabled(),
        ResourceMetricsConfig::disabled(),
        TraceFinalizationConfig::default(),
        WorkloadDiagnostics::default(),
        RuntimeExportConfig::disabled(),
        EnforcementConfig::disabled(),
        CommandControlConfig::disabled(),
        NetworkControlConfig::disabled(),
    )
    .unwrap();
    let mut host = DaemonServiceHost::new(wiring);

    let error = host
        .handle(ControlCommand::TrackAdd(TrackAddCommand {
            request_id: RequestId::new(1),
            root: test_process_ref(std::process::id()),
            display_name: TraceName::new("direct-derived-profile"),
            profile_name: ProfileName::new("snapshot-ebpf-off-notify-off"),
            tags: BTreeSet::new(),
            launch_mode: false,
            initial_suppressed_fds: Vec::new(),
        }))
        .unwrap_err();

    assert_eq!(error.code, "launch_admission");
}

#[test]
fn launch_mode_suppresses_wrapper_bootstrap_gap() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-launch-bootstrap-gap-test-{}.sqlite",
        std::process::id()
    ));
    let mut profiles = DaemonProfileRegistry::new();
    profiles.insert_capture_profile(CaptureProfile::new(
        ProfileName::new("snapshot"),
        Vec::new(),
    ));
    let wiring = build_runtime_wiring(
        &test_storage_config(storage_path.clone()),
        profiles,
        ebpf_config(false),
        payload_config(false),
        DEFAULT_ACTIVE_TRACE_MAX,
        DiagnosticLogLevel::Info,
        SeccompNotifyConfig::disabled(),
        ProcessSeccompConfig::disabled(),
        AgentInvocationConfig::disabled(),
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        ApplicationProtocolConfig::disabled(),
        ResourceMetricsConfig::disabled(),
        TraceFinalizationConfig::default(),
        WorkloadDiagnostics::default(),
        RuntimeExportConfig::disabled(),
        EnforcementConfig::disabled(),
        CommandControlConfig::disabled(),
        NetworkControlConfig::disabled(),
    )
    .unwrap();
    let mut host = DaemonServiceHost::new(wiring);

    let reply = host
        .handle(ControlCommand::TrackAdd(TrackAddCommand {
            request_id: RequestId::new(1),
            root: test_process_ref(std::process::id()),
            display_name: TraceName::new("launch-wrapper"),
            profile_name: ProfileName::new("snapshot"),
            tags: BTreeSet::new(),
            launch_mode: true,
            initial_suppressed_fds: Vec::new(),
        }))
        .unwrap();
    let control_contract::reply::ControlReply::TrackAdded(reply) = reply else {
        panic!("unexpected reply");
    };

    let list = host
        .handle(ControlCommand::ListTraces(ListTracesCommand {
            request_id: RequestId::new(2),
            selector: None,
        }))
        .unwrap();
    let control_contract::reply::ControlReply::TraceList(items) = list else {
        panic!("unexpected list reply");
    };
    let trace = items
        .iter()
        .find(|item| item.trace_id == reply.trace_id)
        .expect("launch trace");
    assert_eq!(trace.health, TraceHealth::Clean);

    let storage = storage_factory::open_storage_backend(
        &test_storage_config(storage_path.clone()),
        storage_core::StorageOpenMode::ReadOnly,
    )
    .unwrap();
    assert!(storage.list_diagnostics(reply.trace_id).unwrap().is_empty());
}

#[test]
fn resource_metrics_sampler_persists_procfs_samples() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-resource-metrics-test-{}.sqlite",
        std::process::id()
    ));
    let profiles = DaemonProfileRegistry::new();
    let mut wiring = build_runtime_wiring(
        &test_storage_config(storage_path.clone()),
        profiles,
        ebpf_config(false),
        payload_config(false),
        DEFAULT_ACTIVE_TRACE_MAX,
        DiagnosticLogLevel::Info,
        SeccompNotifyConfig::disabled(),
        ProcessSeccompConfig::disabled(),
        AgentInvocationConfig::disabled(),
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        ApplicationProtocolConfig::disabled(),
        ResourceMetricsConfig {
            enabled: true,
            interval_ms: RESOURCE_TEST_INTERVAL.as_millis() as u64,
            include_children: true,
            include_system: true,
            cpu_alert_percent_millis: None,
            memory_alert_rss_kb: None,
        },
        TraceFinalizationConfig::default(),
        WorkloadDiagnostics::default(),
        RuntimeExportConfig::disabled(),
        EnforcementConfig::disabled(),
        CommandControlConfig::disabled(),
        NetworkControlConfig::disabled(),
    )
    .unwrap();

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let process = ProcessIdentity::new(std::process::id(), 1, 1);
    create_active_trace(
        &mut wiring,
        trace_id,
        process.clone(),
        ProfileName::new("resource-metrics"),
        TraceName::new("resource-metrics"),
        vec![CapabilityRequest::new(
            Capability::ResourceMetrics,
            RequestMode::Required,
        )],
        RESOURCE_METRICS_COLLECTOR,
        vec![Capability::ResourceMetrics],
    );
    std::thread::sleep(RESOURCE_TEST_INTERVAL + RESOURCE_TEST_INTERVAL);
    wiring
        .attach_service
        .drain_live_events_impl(&mut wiring.trace_runtime)
        .unwrap();

    let events = wiring.attach_service.storage.list_events(trace_id).unwrap();
    let resource = events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::Resource(payload) => Some(payload),
            _ => None,
        })
        .expect("resource sample event");
    assert_eq!(resource.scope, "process_tree");
    assert_eq!(resource.subject, format!("pid:{}", std::process::id()));
    assert!(resource.rss_kb.unwrap_or_default() > 0);
    assert!(resource.virtual_memory_kb.unwrap_or_default() >= resource.rss_kb.unwrap_or_default());
    assert!(resource.metadata.contains_key("comm"));
    assert!(resource.metadata.contains_key("host_mem_total_kb"));
    assert!(resource.metadata.contains_key("host_loadavg_1m"));
    assert_eq!(
        resource
            .metadata
            .get("sampled_processes")
            .map(String::as_str),
        Some("1")
    );
}

#[test]
fn storage_retention_purges_expired_terminal_trace() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-retention-expired-test-{}.sqlite",
        std::process::id()
    ));
    let mut wiring = retention_wiring(storage_path, retention_test_config());
    let trace_id = model_core::ids::TraceId::new(1);
    let trace = terminal_trace(
        trace_id,
        UNIX_EPOCH + Duration::from_secs(1),
        BTreeSet::new(),
    );
    wiring
        .attach_service
        .storage
        .create_trace(trace)
        .expect("write expired trace");

    wiring
        .attach_service
        .drain_live_events_impl(&mut wiring.trace_runtime)
        .expect("run retention sweep");

    assert!(
        wiring
            .attach_service
            .storage
            .list_traces(&TraceFilter::default())
            .expect("list traces")
            .is_empty()
    );
    assert!(wiring.attach_service.storage.get_trace(trace_id).is_err());
}

#[test]
fn storage_retention_keeps_recent_terminal_trace() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-retention-recent-test-{}.sqlite",
        std::process::id()
    ));
    let mut wiring = retention_wiring(storage_path, retention_test_config());
    let trace_id = model_core::ids::TraceId::new(1);
    let trace = terminal_trace(trace_id, SystemTime::now(), BTreeSet::new());
    wiring
        .attach_service
        .storage
        .create_trace(trace)
        .expect("write recent trace");

    wiring
        .attach_service
        .drain_live_events_impl(&mut wiring.trace_runtime)
        .expect("run retention sweep");

    assert_eq!(
        wiring
            .attach_service
            .storage
            .list_traces(&TraceFilter::default())
            .expect("list traces")
            .len(),
        1
    );
}

#[test]
fn storage_retention_keeps_active_and_protected_traces() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-retention-protected-test-{}.sqlite",
        std::process::id()
    ));
    let mut wiring = retention_wiring(storage_path, retention_test_config());
    let active_trace_id = model_core::ids::TraceId::new(1);
    let protected_trace_id = model_core::ids::TraceId::new(2);
    let active_trace = TraceRecord::new(
        active_trace_id,
        ProcessIdentity::new(100, 1, 1),
        TraceName::new("active"),
        ProfileName::new("snapshot"),
        UNIX_EPOCH,
    );
    let mut tags = BTreeSet::new();
    tags.insert("retain".to_string());
    let protected_trace = terminal_trace(
        protected_trace_id,
        UNIX_EPOCH + Duration::from_secs(1),
        tags,
    );
    wiring
        .attach_service
        .storage
        .create_trace(active_trace)
        .expect("write active trace");
    wiring
        .attach_service
        .storage
        .create_trace(protected_trace)
        .expect("write protected trace");

    wiring
        .attach_service
        .drain_live_events_impl(&mut wiring.trace_runtime)
        .expect("run retention sweep");

    let traces = wiring
        .attach_service
        .storage
        .list_traces(&TraceFilter::default())
        .expect("list traces");
    assert_eq!(traces.len(), 2);
    assert!(traces.iter().any(|trace| trace.trace_id == active_trace_id));
    assert!(
        traces
            .iter()
            .any(|trace| trace.trace_id == protected_trace_id)
    );
}

pub(super) fn test_storage_config(path: std::path::PathBuf) -> StorageConfig {
    StorageConfig::sqlite_path(path)
}

pub(super) fn seccomp_notify_disabled() -> SeccompNotifyConfig {
    SeccompNotifyConfig::disabled()
}

pub(super) fn process_seccomp_disabled() -> ProcessSeccompConfig {
    ProcessSeccompConfig::disabled()
}

pub(super) fn agent_invocation_disabled() -> AgentInvocationConfig {
    AgentInvocationConfig::disabled()
}

pub(super) fn application_protocol_disabled() -> ApplicationProtocolConfig {
    ApplicationProtocolConfig::disabled()
}

pub(super) fn resource_metrics_disabled() -> ResourceMetricsConfig {
    ResourceMetricsConfig::disabled()
}

pub(super) fn workload_diagnostics_disabled() -> WorkloadDiagnostics {
    WorkloadDiagnostics::default()
}

pub(super) fn export_runtime_disabled() -> RuntimeExportConfig {
    RuntimeExportConfig::disabled()
}

pub(super) fn enforcement_disabled() -> EnforcementConfig {
    EnforcementConfig::disabled()
}

pub(super) fn network_control_disabled() -> NetworkControlConfig {
    NetworkControlConfig::disabled()
}

fn retention_test_config() -> StorageRetentionConfig {
    StorageRetentionConfig {
        enabled: true,
        max_trace_age: Duration::from_secs(2 * 60),
        sweep_interval: Duration::from_millis(1),
        min_terminal_age: Duration::from_secs(1),
        max_traces_per_sweep: 10,
        protected_tags: vec!["retain".to_string(), "pinned".to_string()],
        checkpoint_after_sweep: true,
    }
}

fn retention_wiring(
    storage_path: std::path::PathBuf,
    storage_retention: StorageRetentionConfig,
) -> crate::runtime_wiring::DaemonRuntimeWiring<super::attach::StorageAttachService> {
    cleanup_storage_files(&storage_path);
    build_runtime_wiring_with_storage_retention(
        &test_storage_config(storage_path),
        DaemonProfileRegistry::new(),
        ebpf_config(false),
        payload_config(false),
        DEFAULT_ACTIVE_TRACE_MAX,
        DiagnosticLogLevel::Info,
        SeccompNotifyConfig::disabled(),
        ProcessSeccompConfig::disabled(),
        AgentInvocationConfig::disabled(),
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        ApplicationProtocolConfig::disabled(),
        ResourceMetricsConfig::disabled(),
        storage_retention,
        TraceFinalizationConfig::default(),
        WorkloadDiagnostics::default(),
        RuntimeExportConfig::disabled(),
        EnforcementConfig::disabled(),
        CommandControlConfig::disabled(),
        NetworkControlConfig::disabled(),
    )
    .expect("build retention test wiring")
}

fn cleanup_storage_files(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

fn terminal_trace(
    trace_id: model_core::ids::TraceId,
    completed_at: SystemTime,
    tags: BTreeSet<String>,
) -> TraceRecord {
    let mut trace = TraceRecord::new(
        trace_id,
        ProcessIdentity::new(100, 1, 1),
        TraceName::new(format!("trace-{}", trace_id.get())),
        ProfileName::new("snapshot"),
        completed_at,
    );
    trace.lifecycle_state = TraceLifecycleState::Completed;
    trace.timings.completed_at = Some(completed_at);
    trace.tags = tags;
    trace
}

fn create_active_trace(
    wiring: &mut crate::runtime_wiring::DaemonRuntimeWiring<super::attach::StorageAttachService>,
    trace_id: model_core::ids::TraceId,
    process: ProcessIdentity,
    profile_name: ProfileName,
    display_name: TraceName,
    capability_requests: Vec<CapabilityRequest>,
    collector_name: &str,
    collector_capabilities: Vec<Capability>,
) {
    let profile = CaptureProfile::new(profile_name.clone(), capability_requests);
    let captured_at = SystemTime::UNIX_EPOCH;
    let profile_snapshot = CaptureProfileSnapshot::from_profile(&profile, captured_at);
    wiring
        .trace_runtime
        .create_starting_trace(
            trace_id,
            TrackTraceRequest {
                root_identity: process,
                root_container_id: None,
                display_name,
                profile_snapshot,
                tags: BTreeSet::new(),
                created_at: captured_at,
            },
            SensorPlan {
                profile_name,
                collectors: vec![CollectorPlan {
                    collector_name: CollectorName::new(collector_name),
                    capabilities: collector_capabilities,
                }],
                unbound_opportunistic: Vec::new(),
            },
        )
        .unwrap();
    wiring
        .trace_runtime
        .activate_trace(trace_id, captured_at)
        .unwrap();
    let entry = wiring.trace_runtime.get_trace(trace_id).unwrap().clone();
    wiring
        .attach_service
        .storage
        .create_trace(entry.trace)
        .unwrap();
    for membership in entry.memberships.memberships().cloned() {
        wiring
            .attach_service
            .storage
            .upsert_membership(membership)
            .unwrap();
    }
}

fn ebpf_config(enabled: bool) -> EbpfCollectorConfig {
    EbpfCollectorConfig {
        enabled_mode: if enabled {
            EbpfEnabledMode::True
        } else {
            EbpfEnabledMode::False
        },
        enabled,
        memlock_rlimit: MemlockRlimit::Inherit,
        tracked_process_max_entries: 64,
        pending_operation_max_entries: 128,
        suppressed_fd_max_entries: 128,
        suppressed_fd_index_slots_per_process: 64,
        event_ring_buffer_max_bytes: 4096,
        file_path_capture_enabled: false,
        file_path_max_bytes: 255,
    }
}

fn default_operator_config() -> OperatorConfig {
    let raw =
        OperatorConfig::default_hierarchical_template().expect("operator config template renders");
    OperatorConfig::parse(&raw).expect("operator config template parses")
}

fn payload_config(tls_enabled: bool) -> PayloadConfig {
    let mut payload = default_operator_config().payload_config;
    payload.tls.enabled = tls_enabled;
    payload.tls.sync_event_socket_path = std::env::temp_dir().join(format!(
        "actrail-test-tls-sync-{}-{}.sock",
        std::process::id(),
        TEST_SOCKET_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    payload
}
