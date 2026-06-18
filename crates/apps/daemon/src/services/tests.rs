//! Integration-oriented daemon service tests.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use config_core::capture_profile::CaptureProfile;
use config_core::daemon::{
    AgentInvocationConfig, ApplicationProtocolConfig, DiagnosticLogLevel, EbpfCollectorConfig,
    EnforcementBackend, EnforcementConfig, EnforcementDecision, EnforcementMarkStrategy,
    EnforcementScope, FileObservationConfig, MemlockRlimit, OPERATOR_CONFIG_TEMPLATE,
    OperatorConfig, PayloadConfig, ProcessSeccompConfig, ProcessSeccompSyscall,
    ResourceMetricsConfig, RuntimeExportConfig, SeccompNotifyConfig, SemanticRetentionConfig,
    SseDataPolicy,
};
use config_core::trace_snapshot::CaptureProfileSnapshot;
use control_contract::command::{ControlCommand, ListTracesCommand, TrackAddCommand};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::event::EventPayload;
use model_core::ids::{CollectorName, ProfileName, RequestId, TraceName};
use model_core::process::ProcessIdentity;
use model_core::trace::TraceHealth;
use storage_factory::StorageConfig;
use trace_runtime::commands::TrackTraceRequest;
use trace_runtime::sensor_plan::{CollectorPlan, SensorPlan};
use uds_control_server::ControlService;

use crate::profiles::DaemonProfileRegistry;
use crate::service_host::DaemonServiceHost;

use super::build_runtime_wiring;

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
        DiagnosticLogLevel::Info,
        seccomp_notify_disabled(),
        process_seccomp_disabled(),
        agent_invocation_disabled(),
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        application_protocol_disabled(),
        resource_metrics_disabled(),
        workload_diagnostics_disabled(),
        export_runtime_disabled(),
        enforcement_disabled(),
    )
    .unwrap();
    let mut host = DaemonServiceHost::new(wiring);

    let reply = host
        .handle(ControlCommand::TrackAdd(TrackAddCommand {
            request_id: RequestId::new(1),
            root_pid: std::process::id(),
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
        DiagnosticLogLevel::Info,
        seccomp_notify_disabled(),
        process_seccomp_disabled(),
        agent_invocation_disabled(),
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        application_protocol_disabled(),
        resource_metrics_disabled(),
        workload_diagnostics_disabled(),
        export_runtime_disabled(),
        enforcement_disabled(),
    )
    .unwrap();
    let mut host = DaemonServiceHost::new(wiring);

    let reply = host
        .handle(ControlCommand::TrackAdd(TrackAddCommand {
            request_id: RequestId::new(1),
            root_pid: std::process::id(),
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
        DiagnosticLogLevel::Info,
        seccomp_notify_disabled(),
        process_seccomp_disabled(),
        agent_invocation_disabled(),
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        application_protocol_disabled(),
        ResourceMetricsConfig {
            enabled: true,
            interval_ms: RESOURCE_TEST_INTERVAL.as_millis() as u64,
            include_children: true,
            include_system: true,
            cpu_alert_percent_millis: None,
            memory_alert_rss_kb: None,
        },
        workload_diagnostics_disabled(),
        export_runtime_disabled(),
        enforcement_disabled(),
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

pub(super) fn test_storage_config(path: std::path::PathBuf) -> StorageConfig {
    StorageConfig::sqlite_path(path)
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

fn payload_config(tls_enabled: bool) -> PayloadConfig {
    let mut payload = OperatorConfig::parse(OPERATOR_CONFIG_TEMPLATE)
        .expect("operator config template parses")
        .payload_config;
    payload.tls.enabled = tls_enabled;
    payload.tls.sync_event_socket_path = std::env::temp_dir().join(format!(
        "actrail-test-tls-sync-{}-{}.sock",
        std::process::id(),
        TEST_SOCKET_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    payload
}

fn seccomp_notify_disabled() -> SeccompNotifyConfig {
    SeccompNotifyConfig {
        enabled: false,
        reserved_listener_fd: 253,
    }
}

fn process_seccomp_disabled() -> ProcessSeccompConfig {
    ProcessSeccompConfig {
        enabled: false,
        syscalls: vec![
            ProcessSeccompSyscall::Execve,
            ProcessSeccompSyscall::Execveat,
            ProcessSeccompSyscall::Fork,
            ProcessSeccompSyscall::Vfork,
            ProcessSeccompSyscall::Clone,
            ProcessSeccompSyscall::Clone3,
        ],
        max_args: 64,
        max_arg_bytes: 4096,
        pending_max_entries: 128,
    }
}

fn agent_invocation_disabled() -> AgentInvocationConfig {
    AgentInvocationConfig {
        enabled: false,
        commands: vec![
            "opencode".to_string(),
            ".opencode".to_string(),
            "claude".to_string(),
        ],
    }
}

fn application_protocol_disabled() -> ApplicationProtocolConfig {
    ApplicationProtocolConfig {
        enabled: false,
        http1_enabled: false,
        http2_enabled: false,
        capture_host: false,
        sse_enabled: false,
        sse_data_policy: SseDataPolicy::Disabled,
        sse_max_buffer_bytes: TEST_HTTP_BUFFER_BYTES,
        sse_max_data_bytes: TEST_HTTP_BUFFER_BYTES,
        http2_max_frame_bytes: TEST_HTTP2_MAX_FRAME_BYTES,
        http2_max_connection_buffer_bytes: TEST_HTTP_BUFFER_BYTES,
        http2_emit_data_preview: false,
        http2_max_data_preview_bytes: TEST_HTTP2_PREVIEW_BYTES,
    }
}

fn resource_metrics_disabled() -> ResourceMetricsConfig {
    ResourceMetricsConfig {
        enabled: false,
        interval_ms: 1,
        include_children: true,
        include_system: true,
        cpu_alert_percent_millis: None,
        memory_alert_rss_kb: None,
    }
}

fn workload_diagnostics_disabled() -> super::workload_diagnostics::WorkloadDiagnostics {
    super::workload_diagnostics::WorkloadDiagnostics::default()
}

fn export_runtime_disabled() -> RuntimeExportConfig {
    let mut config = OperatorConfig::parse(OPERATOR_CONFIG_TEMPLATE)
        .unwrap()
        .export_runtime;
    config.enabled = false;
    config
}

fn enforcement_disabled() -> EnforcementConfig {
    EnforcementConfig {
        enabled: false,
        backend: EnforcementBackend::Fanotify,
        scope: EnforcementScope::Trace,
        rules_path: std::env::temp_dir().join("actrail-enforcement-disabled.conf"),
        default_decision: EnforcementDecision::Allow,
        mark_strategy: EnforcementMarkStrategy::ParentDirectories,
        audit_enabled: true,
        event_buffer_bytes: 4096,
    }
}
