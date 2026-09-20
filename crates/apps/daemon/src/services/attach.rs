//! Attach service backed by procfs bootstrap and storage persistence.

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant, SystemTime};

#[path = "attach/debug.rs"]
mod debug;
#[path = "attach/factory.rs"]
mod factory;
#[path = "attach/helpers.rs"]
mod helpers;
#[path = "attach/logging.rs"]
mod logging;
#[path = "attach/plugin_config.rs"]
mod plugin_config;
#[path = "attach/plugin_configuration.rs"]
mod plugin_configuration;
#[path = "attach/plugins.rs"]
mod plugins;
#[path = "attach/preflight.rs"]
mod preflight;
#[path = "attach/service.rs"]
mod service;

use collector_binding::TraceBindingRequest;
use collector_instance::CollectorInstance;
use config_core::capture_profile::{
    DeploymentPermissionAvailability, DeploymentPermissionPolicy, LaunchSeccompRequirements,
    PermissionMode, resolve_deployment_permissions,
};
use config_core::daemon::{
    DiagnosticLogLevel, FileObservationConfig, PayloadRedactionPolicy, PayloadStdioStorageMode,
    SemanticRetentionConfig,
};
use config_core::trace_snapshot::CaptureProfileSnapshot;
use control_contract::command::{
    DeploymentPermissionMode, LaunchTlsProbePlan, ProcessRef, ResolveLaunchPermissionsCommand,
    ResolveLaunchTlsPlanCommand, TrackAddCommand,
};
use control_contract::reply::{
    ControlError, LaunchPermissionsReply, LaunchTlsPlanReply, PluginCommandReply, TrackAddReply,
};
use ebpf_collector::EbpfCollector;
use ebpf_collector::loader::DynamicTlsProbePlan;
use ebpf_collector::procfs::{
    ProcfsIdentityReader, ProcfsTreeSnapshotter, read_container_identity, resolve_namespaced_pid,
};
use export_core::ExportRuntime;
use model_core::capability::Capability;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessObservation, ProcessRecord};
use model_core::resource_scope::TraceResourceScope;
use plugin_system::PluginInstanceStatus;
use process_identity::ProcessIdentityError;
use process_identity::ProcessIdentityManager;
use provider_label::ProviderClassifier;
use recording_runtime::RecordingWriter;
use semantic_action_runtime::LiveSemanticActionRuntime;
use storage_core::StorageBackend;
use trace_runtime::commands::TrackTraceRequest;
use trace_runtime::sensor_plan::SensorPlan;

use crate::profiles::DaemonProfileRegistry;
use crate::service_host::AttachService;
use crate::services::alert_forwarding::AlertForwardingService;
use crate::services::alert_ingress::AlertIngress;
use crate::services::application_protocol::ApplicationProtocolAnalyzer;
use crate::services::command_control::CommandControlService;
use crate::services::control_runtime::ControlPluginRuntime;
use crate::services::enforcement::FanotifyEnforcementService;
use crate::services::network_control::NetworkControlService;
use crate::services::payload_gate::{PayloadBodyRetentionGate, SocketHttpPayloadGate};
use crate::services::post_trace::{PostTraceBroker, PostTraceCoordinator};
use crate::services::process_seccomp::{ProcessSeccompObservation, ProcessSeccompService};
use crate::services::resource_metrics::ResourceMetricsSampler;
use crate::services::retention::StorageRetentionService;
use crate::services::seccomp_notify::SeccompNotifyService;
use crate::services::seccomp_socket::SeccompSocketService;
use crate::services::seccomp_tls::SeccompTlsService;
use crate::services::tls_sync::{PinnedRuntimePath, RuntimeRootPathMapper, TlsSyncService};
use crate::services::workload_diagnostics::WorkloadDiagnostics;

use self::helpers::{capability_requested, collector_capability_requests};

pub(crate) struct StorageAttachService {
    pub(super) profiles: DaemonProfileRegistry,
    /// Host/VM id (OTel `host.id`) stamped onto every trace this daemon emits.
    pub(super) host_id: Option<String>,
    pub(super) launch_seccomp_requirements: LaunchSeccompRequirements,
    pub(super) storage: Box<dyn StorageBackend>,
    pub(super) process_registry: ProcessIdentityManager,
    pub(super) process_id_block_size: u64,
    pub(super) collector: EbpfCollector,
    pub(super) host_ebpf_preflight:
        BTreeMap<model_core::ids::ProfileName, preflight::EbpfPreflightReport>,
    pub(super) identity_reader: ProcfsIdentityReader,
    pub(super) snapshotter: ProcfsTreeSnapshotter,
    pub(super) next_event_id: u64,
    pub(super) next_diagnostic_id: u64,
    pub(super) next_payload_segment_id: u64,
    pub(super) payload_tls_enabled: bool,
    pub(super) diagnostic_log_level: DiagnosticLogLevel,
    pub(super) last_payload_tls_diagnostics: Option<String>,
    pub(super) payload_tls_redaction_policy: PayloadRedactionPolicy,
    pub(super) payload_stdio_enabled: bool,
    pub(super) payload_stdio_redaction_policy: PayloadRedactionPolicy,
    pub(super) payload_stdio_stdin_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_stdio_stdout_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_stdio_stderr_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_socket_enabled: bool,
    pub(super) payload_socket_redaction_policy: PayloadRedactionPolicy,
    pub(super) socket_payload_gate: SocketHttpPayloadGate,
    pub(super) payload_body_retention_gate: PayloadBodyRetentionGate,
    pub(super) payload_reorderer: crate::services::payload::reorder::PayloadSegmentReorderer,
    pub(super) seccomp_notify: SeccompNotifyService,
    pub(super) seccomp_tls: SeccompTlsService,
    pub(super) tls_sync: TlsSyncService,
    pub(super) seccomp_socket: SeccompSocketService,
    pub(super) process_seccomp: ProcessSeccompService,
    pub(super) command_control: CommandControlService,
    pub(super) network_control: NetworkControlService,
    pub(super) pending_process_seccomp_observations: Vec<ProcessSeccompObservation>,
    pub(super) semantic_retention: SemanticRetentionConfig,
    pub(super) file_observation: FileObservationConfig,
    pub(super) application_protocol: ApplicationProtocolAnalyzer,
    pub(super) resource_metrics: ResourceMetricsSampler,
    pub(super) storage_retention: StorageRetentionService,
    pub(super) enforcement: FanotifyEnforcementService,
    pub(super) control_plugins: ControlPluginRuntime,
    plugin_configs: plugin_config::PluginConfigManager,
    pub(super) semantic_actions: LiveSemanticActionRuntime,
    pub(super) export_runtime: ExportRuntime,
    pub(super) alert_ingress: AlertIngress,
    pub(super) idle_detection: idle_detector::IdleDetector,
    pub(super) agent_executions: agent_host::ExecutionStates,
    pub(super) idle_detection_alert_host: Option<std::sync::Arc<dyn plugin_system::AlertHost>>,
    pub(super) alert_forwarding: AlertForwardingService,
    pub(super) post_trace_broker: PostTraceBroker,
    pub(super) post_trace_coordinator: PostTraceCoordinator,
    pub(super) workload_diagnostics: WorkloadDiagnostics,
    pub(super) finalized_terminal_traces: BTreeSet<model_core::ids::TraceId>,
    pub(super) pending_terminal_finalizations: BTreeSet<model_core::ids::TraceId>,
    pub(super) terminal_finalization_queued_at: BTreeMap<model_core::ids::TraceId, Instant>,
    pub(super) finalization_traces_per_cycle: usize,
    pub(super) finalization_poll_interval: Duration,
    pub(super) terminal_settle_delay: Duration,
    pub(super) finalization_shutdown_drain_timeout: Duration,
    pub(super) shutdown_runtime_timeout: Duration,
    pub(super) diagnosed_terminal_open_memberships:
        BTreeSet<(model_core::ids::TraceId, ProcessIdentity)>,
    pub(super) provider_classifier: Box<dyn ProviderClassifier>,
    pub(super) provider_classification_enabled: bool,
    /// Cross-batch tool name cache: trace_id → (llm_call_action_id → tool_name).
    /// Populated when LlmResponse actions arrive; consumed by propagate_tool_names_to_commands
    /// to retroactively set command.tool.name on CommandInvocations that were persisted
    /// before their corresponding LlmResponse was processed.
    pub(super) pending_tool_names: BTreeMap<TraceId, BTreeMap<String, String>>,
}
impl StorageAttachService {
    pub(super) fn resolve_process_observation(
        &mut self,
        observation: ProcessObservation,
    ) -> Result<(ProcessIdentity, Option<ProcessRecord>), ControlError> {
        let resolution = loop {
            match self.process_registry.resolve_or_create(observation.clone()) {
                Ok(resolution) => break resolution,
                Err(ProcessIdentityError::IdBlockExhausted) => {
                    let block_size = factory::process_id_block_size()?;
                    let (block_start, block_end) = self
                        .storage
                        .reserve_process_id_block(block_size)
                        .map_err(|error| ControlError::new(error.stage, error.message))?;
                    self.process_registry
                        .install_reserved_block(block_start, block_end)
                        .map_err(|error| {
                            ControlError::new("process_id_block_install", format!("{error:?}"))
                        })?;
                }
                Err(error) => {
                    return Err(ControlError::new(
                        "process_identity_resolution",
                        format!("{error:?}"),
                    ));
                }
            }
        };
        let changed = resolution.created || resolution.enriched;
        let record = changed
            .then(|| self.process_registry.record(resolution.identity).cloned())
            .flatten();
        Ok((resolution.identity, record))
    }

    pub(crate) fn collector_name(&self) -> String {
        self.collector.descriptor().name.to_string()
    }

    pub(crate) fn collector_ready(&self) -> bool {
        self.collector.probe_result().reason_unavailable.is_none()
    }

    pub(crate) fn any_host_ebpf_preflight_available(&self) -> bool {
        self.host_ebpf_preflight
            .values()
            .any(|report| report.available)
    }

    pub(crate) fn collector_descriptor(&self) -> collector_capability::CollectorDescriptor {
        self.collector.descriptor().clone()
    }

    pub(crate) fn set_id_seeds(&mut self, next_event_id: u64, next_diagnostic_id: u64) {
        self.next_event_id = next_event_id;
        self.next_diagnostic_id = next_diagnostic_id;
    }

    pub(crate) fn set_payload_segment_id_seed(&mut self, next_payload_segment_id: u64) {
        self.next_payload_segment_id = next_payload_segment_id;
    }

    fn bootstrap_snapshot(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        profile_snapshot: CaptureProfileSnapshot,
        sensor_plan: SensorPlan,
    ) -> Result<BootstrapSnapshot, ControlError> {
        let root_observation = resolve_process_ref(&command.root)?;
        let root_host_pid = root_observation
            .host
            .as_ref()
            .map(|host| host.pid)
            .ok_or_else(|| ControlError::new("pid_resolution", "root host PID is missing"))?;
        let (root_identity, root_record) =
            self.resolve_process_observation(root_observation.clone())?;
        let root_pid_namespace = root_observation
            .namespace
            .as_ref()
            .map(|coordinates| coordinates.pid_namespace.clone());
        // Resolve the container identity host-side from the already-resolved
        // host pid. This sees the full runtime cgroup path; a container-local
        // `/proc/self/cgroup` may be masked to `0::/`). `None` = host process
        // or unrecognized runtime layout. The pod UID (k8s.pod.uid) comes from
        // the same parse and is `None` outside kubepods cgroups.
        let (root_container_id, root_pod_uid) = match read_container_identity(root_host_pid) {
            Some(identity) => (Some(identity.container_id), identity.pod_uid),
            None => (None, None),
        };
        let snapshot = process_tree_snapshot_contract::snapshot::ProcessTreeSnapshotter::snapshot(
            &self.snapshotter,
            &root_observation,
        )
        .map_err(|error| ControlError::new("snapshot", error))?;
        let root_working_directory = snapshot.root_working_directory().map(str::to_string);

        let trace_id = trace_runtime.reserve_trace_id();
        trace_runtime
            .create_starting_trace(
                trace_id,
                TrackTraceRequest {
                    root_identity: root_identity.clone(),
                    root_pid_namespace,
                    root_container_id,
                    root_pod_uid,
                    root_host_id: self.host_id.clone(),
                    root_working_directory,
                    display_name: command.display_name.clone(),
                    profile_snapshot,
                    tags: command.tags.clone(),
                    created_at: SystemTime::now(),
                },
                sensor_plan,
            )
            .map_err(|error| ControlError::new("create_trace", format!("{:?}", error)))?;

        let mut process_records = BTreeMap::new();
        if let Some(record) = root_record {
            process_records.insert(record.identity, record);
        }
        let mut bootstrap_partial = false;
        for process in snapshot.processes {
            let (identity, record) = self.resolve_process_observation(process.identity)?;
            if let Some(record) = record {
                process_records.insert(record.identity, record);
            }
            if identity == root_identity {
                continue;
            }
            let Some(parent_observation) = process.parent else {
                bootstrap_partial = true;
                continue;
            };
            let (parent, parent_record) = self.resolve_process_observation(parent_observation)?;
            if let Some(record) = parent_record {
                process_records.insert(record.identity, record);
            }
            let membership = model_core::process::ProcessMembership::inherited(
                trace_id,
                identity,
                parent,
                snapshot.captured_at,
            );
            trace_runtime
                .insert_membership(trace_id, membership)
                .map_err(|error| ControlError::new("insert_membership", format!("{:?}", error)))?;
        }
        Ok(BootstrapSnapshot {
            trace_id,
            root_identity,
            root_observation,
            root_host_pid,
            process_records: process_records.into_values().collect(),
            diagnostic_kind: if bootstrap_partial {
                DiagnosticKind::BootstrapPartial
            } else {
                DiagnosticKind::BootstrapGap
            },
        })
    }

    fn attach_snapshot_only(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        profile_snapshot: CaptureProfileSnapshot,
        sensor_plan: SensorPlan,
    ) -> Result<TrackAddReply, ControlError> {
        let bootstrap =
            self.bootstrap_snapshot(trace_runtime, command, profile_snapshot, sensor_plan)?;
        self.finalize_trace(
            trace_runtime,
            bootstrap.trace_id,
            bootstrap.root_identity,
            bootstrap.process_records,
            command.launch_mode,
            bootstrap.diagnostic_kind,
            "snapshot-only attach completed without eBPF coverage guard",
        )
    }

    fn attach_with_collector(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        profile_snapshot: CaptureProfileSnapshot,
        sensor_plan: SensorPlan,
        launch_pidfd: Option<OwnedFd>,
    ) -> Result<TrackAddReply, ControlError> {
        let collector_name = self.collector_name();
        let requested_capabilities = collector_capability_requests(
            &profile_snapshot.capability_requests,
            &sensor_plan,
            &collector_name,
        );
        let uses_ebpf_collector = !requested_capabilities.is_empty();
        if !uses_ebpf_collector && !command.tls_probe_plans.is_empty() {
            return Err(ControlError::new(
                "attach_dynamic_tls",
                "static TLS plans require the Host eBPF collector",
            ));
        }
        let bootstrap = self.bootstrap_snapshot(
            trace_runtime,
            command,
            profile_snapshot.clone(),
            sensor_plan,
        )?;
        let pinned_tls_plans =
            match self.pin_direct_tls_plans(bootstrap.root_host_pid, &command.tls_probe_plans) {
                Ok(plans) => plans,
                Err(error) => {
                    let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                    return Err(error);
                }
            };

        let pending_resource_scope = {
            let entry = trace_runtime.get_trace(bootstrap.trace_id).ok_or_else(|| {
                ControlError::new("trace_missing", "trace disappeared during bootstrap")
            })?;
            let root_pid = bootstrap
                .root_observation
                .host
                .as_ref()
                .map(|host| host.pid)
                .ok_or_else(|| ControlError::new("pid_resolution", "root host PID is missing"))?;
            match self
                .resource_metrics
                .admit_stopped_launch(entry, root_pid, command.launch_mode)
            {
                Ok(scope) => scope,
                Err(error) => {
                    let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                    return Err(error);
                }
            }
        };

        let member_processes = trace_runtime
            .get_trace(bootstrap.trace_id)
            .ok_or_else(|| {
                ControlError::new("trace_missing", "trace disappeared during bootstrap")
            })?
            .memberships
            .memberships()
            .map(|membership| {
                self.process_registry
                    .record(membership.identity)
                    .cloned()
                    .ok_or_else(|| {
                        ControlError::new(
                            "process_registry",
                            format!("missing process record {}", membership.identity.get()),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        if uses_ebpf_collector {
            let binding_request = TraceBindingRequest {
                trace_id: bootstrap.trace_id,
                root_identity: bootstrap.root_identity,
                root_observation: bootstrap.root_observation.clone(),
                root_namespace_pid: command.root.namespace_pid,
                profile_snapshot: profile_snapshot.clone(),
                requested_capabilities,
                initial_suppressed_fds: command.initial_suppressed_fds.clone(),
            };
            let bind_result = if command.launch_mode {
                let pidfd = launch_pidfd.ok_or_else(|| {
                    ControlError::new(
                        "launch_pidfd",
                        "eBPF launch attach requires an SCM_RIGHTS pidfd",
                    )
                })?;
                self.collector.bind_launch_trace(&binding_request, pidfd)
            } else {
                self.collector.bind_trace(&binding_request)
            };
            if let Err(error) = bind_result {
                let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                return Err(ControlError::new(error.stage, error.message));
            }

            for plan in &pinned_tls_plans {
                let dynamic_plan = plan.dynamic_plan();
                if let Err(error) = self.collector.attach_dynamic_tls_plan(&dynamic_plan) {
                    if command.launch_mode {
                        let _ = self.collector.unbind_trace(bootstrap.trace_id);
                    }
                    let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                    return Err(ControlError::new(error.stage, error.message));
                }
            }

            if let Err(error) = self
                .collector
                .seed_trace_memberships(bootstrap.trace_id, member_processes)
            {
                if command.launch_mode {
                    let _ = self.collector.unbind_trace(bootstrap.trace_id);
                }
                let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                return Err(ControlError::new(error.stage, error.message));
            }
        }

        if pending_resource_scope.is_none() && !command.launch_mode {
            if let Some(coordinates) = bootstrap.root_observation.host.as_ref() {
                let entry = trace_runtime.get_trace(bootstrap.trace_id).ok_or_else(|| {
                    ControlError::new("trace_missing", "trace disappeared during bootstrap")
                })?;
                if let Err(error) = self
                    .resource_metrics
                    .admit_external_container(entry, coordinates)
                {
                    if uses_ebpf_collector {
                        let _ = self.collector.unbind_trace(bootstrap.trace_id);
                    }
                    let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                    return Err(error);
                }
            }
        }

        let result = self.finalize_trace(
            trace_runtime,
            bootstrap.trace_id,
            bootstrap.root_identity,
            bootstrap.process_records,
            command.launch_mode,
            bootstrap.diagnostic_kind,
            if uses_ebpf_collector {
                "snapshot bootstrap completed before live eBPF tracking and remains gap-marked"
            } else {
                "snapshot bootstrap completed before virtual collector sampling and remains gap-marked"
            },
        );
        if let Err(error) = result {
            if uses_ebpf_collector && command.launch_mode {
                let _ = self.collector.unbind_trace(bootstrap.trace_id);
            }
            let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
            return Err(error);
        }
        if let Some(scope) = pending_resource_scope {
            if let Err(error) = self.persist_and_activate_resource_scope(&scope) {
                if uses_ebpf_collector && command.launch_mode {
                    let _ = self.collector.unbind_trace(bootstrap.trace_id);
                }
                let _ = trace_runtime.fail_trace(bootstrap.trace_id, SystemTime::now());
                let _ = self.persist_trace_state(trace_runtime, bootstrap.trace_id);
                return Err(error);
            }
        }
        result
    }

    fn persist_and_activate_resource_scope(
        &mut self,
        scope: &TraceResourceScope,
    ) -> Result<(), ControlError> {
        self.storage
            .create_resource_scope(scope.clone())
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        self.resource_metrics.activate_scope(scope)
    }

    fn pin_direct_tls_plans(
        &self,
        root_host_pid: u32,
        plans: &[LaunchTlsProbePlan],
    ) -> Result<Vec<PinnedDirectTlsPlan>, ControlError> {
        if plans.is_empty() {
            return Ok(Vec::new());
        }
        let mapper = self.tls_sync.runtime_root_path_mapper(root_host_pid)?;
        plans
            .iter()
            .map(|plan| PinnedDirectTlsPlan::new(&mapper, plan))
            .collect()
    }

    fn attach_command(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        launch_pidfd: Option<OwnedFd>,
    ) -> Result<TrackAddReply, ControlError> {
        if !command.launch_mode && self.profiles.is_launch_only_profile(&command.profile_name) {
            return Err(ControlError::new(
                "launch_admission",
                "deployment-derived profiles require a daemon launch permission admission",
            ));
        }
        let profile = self
            .profiles
            .capture_profile(&command.profile_name)
            .ok_or_else(|| {
                ControlError::new("unknown_profile", "capture profile does not exist")
            })?;
        let profile_snapshot = CaptureProfileSnapshot::from_profile(profile, SystemTime::now());
        if self.seccomp_tls.enabled()
            && !command.launch_mode
            && capability_requested(
                &profile_snapshot.capability_requests,
                &Capability::TlsPlaintextPayload,
            )
        {
            return Err(ControlError::new(
                "payload_tls_backend",
                "TLS plaintext payload capture is only supported by actrailctl launch",
            ));
        }
        if !command.launch_mode
            && capability_requested(
                &profile_snapshot.capability_requests,
                &Capability::EnforcementCommandExecutionSeccomp,
            )
        {
            return Err(ControlError::new(
                "command_control_backend",
                "command execution enforcement is only supported by actrailctl launch",
            ));
        }
        if !command.launch_mode
            && capability_requested(
                &profile_snapshot.capability_requests,
                &Capability::EnforcementNetworkConnectSeccomp,
            )
        {
            return Err(ControlError::new(
                "network_control_backend",
                "network connect enforcement is only supported by actrailctl launch",
            ));
        }
        let sensor_plan = trace_runtime
            .negotiate(&profile_snapshot)
            .map_err(|error| ControlError::new("negotiate", format!("{:?}", error)))?;

        if sensor_plan.collectors.is_empty() && !command.tls_probe_plans.is_empty() {
            return Err(ControlError::new(
                "attach_dynamic_tls",
                "static TLS plans require the Host eBPF collector",
            ));
        }
        if sensor_plan.collectors.is_empty() {
            return self.attach_snapshot_only(
                trace_runtime,
                command,
                profile_snapshot,
                sensor_plan,
            );
        }

        self.attach_with_collector(
            trace_runtime,
            command,
            profile_snapshot,
            sensor_plan,
            launch_pidfd,
        )
    }
}

fn resolve_process_ref(process: &ProcessRef) -> Result<ProcessObservation, ControlError> {
    resolve_namespaced_pid(process.namespace_pid, &process.pid_namespace)
        .map_err(|error| ControlError::new("pid_resolution", error))
}

struct BootstrapSnapshot {
    trace_id: model_core::ids::TraceId,
    root_identity: ProcessIdentity,
    root_observation: ProcessObservation,
    root_host_pid: u32,
    process_records: Vec<ProcessRecord>,
    diagnostic_kind: DiagnosticKind,
}

struct PinnedDirectTlsPlan {
    descriptor: LaunchTlsProbePlan,
    target: PinnedRuntimePath,
    binary: PinnedRuntimePath,
}

impl PinnedDirectTlsPlan {
    fn new(
        mapper: &RuntimeRootPathMapper,
        descriptor: &LaunchTlsProbePlan,
    ) -> Result<Self, ControlError> {
        Ok(Self {
            descriptor: descriptor.clone(),
            target: mapper.pin(&descriptor.target)?,
            binary: mapper.pin(&descriptor.binary)?,
        })
    }

    fn dynamic_plan(&self) -> DynamicTlsProbePlan {
        DynamicTlsProbePlan {
            target: self.target.path().to_path_buf(),
            target_identity: self.descriptor.target_identity.clone(),
            binary: self.binary.path().to_path_buf(),
            binary_identity: self.descriptor.binary_identity.clone(),
            provider: self.descriptor.provider.clone(),
            points: self.descriptor.points.clone(),
        }
    }
}
