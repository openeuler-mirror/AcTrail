//! Attach service backed by procfs bootstrap and storage persistence.

use std::collections::BTreeSet;
use std::os::fd::RawFd;
use std::time::{Duration, SystemTime};

#[path = "attach/debug.rs"]
mod debug;
#[path = "attach/factory.rs"]
mod factory;
#[path = "attach/helpers.rs"]
mod helpers;
#[path = "attach/logging.rs"]
mod logging;

use attach_runtime::snapshot_merge::merge_snapshot;
use collector_binding::TraceBindingRequest;
use collector_instance::CollectorInstance;
use config_core::daemon::{DiagnosticLogLevel, PayloadRedactionPolicy, PayloadStdioStorageMode};
use config_core::trace_snapshot::CaptureProfileSnapshot;
use control_contract::command::TrackAddCommand;
use control_contract::reply::{ControlError, TrackAddReply};
use ebpf_collector::EbpfCollector;
use ebpf_collector::procfs::{ProcfsIdentityReader, ProcfsTreeSnapshotter};
use export_core::ExportRuntime;
use model_core::capability::Capability;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::process::ProcessIdentity;
use provider_label::ProviderClassifier;
use recording_runtime::{RecordingWriter, TraceStateRecord};
use semantic_action_runtime::LiveSemanticActionRuntime;
use storage_core::StorageBackend;
use trace_runtime::commands::TrackTraceRequest;
use trace_runtime::sensor_plan::SensorPlan;

use crate::profiles::DaemonProfileRegistry;
use crate::service_host::AttachService;
use crate::services::application_protocol::ApplicationProtocolAnalyzer;
use crate::services::enforcement::FanotifyEnforcementService;
use crate::services::payload_gate::{PayloadBodyRetentionGate, SocketHttpPayloadGate};
use crate::services::process_seccomp::{ProcessSeccompObservation, ProcessSeccompService};
use crate::services::resource_metrics::ResourceMetricsSampler;
use crate::services::seccomp_notify::SeccompNotifyService;
use crate::services::seccomp_socket::SeccompSocketService;
use crate::services::seccomp_tls::SeccompTlsService;
use crate::services::tls_sync::TlsSyncService;

use self::helpers::{capability_requested, collector_capability_requests};

pub(crate) struct StorageAttachService {
    pub(super) profiles: DaemonProfileRegistry,
    pub(super) storage: Box<dyn StorageBackend>,
    pub(super) collector: EbpfCollector,
    pub(super) identity_reader: ProcfsIdentityReader,
    pub(super) snapshotter: ProcfsTreeSnapshotter,
    pub(super) next_event_id: u64,
    pub(super) next_diagnostic_id: u64,
    pub(super) next_payload_segment_id: u64,
    pub(super) payload_tls_enabled: bool,
    pub(super) diagnostic_log_level: DiagnosticLogLevel,
    pub(super) last_payload_tls_diagnostics: Option<String>,
    pub(super) payload_tls_redaction_policy: PayloadRedactionPolicy,
    pub(super) payload_tls_retention_max_bytes_per_trace: u64,
    pub(super) payload_stdio_enabled: bool,
    pub(super) payload_stdio_redaction_policy: PayloadRedactionPolicy,
    pub(super) payload_stdio_retention_max_bytes_per_trace: u64,
    pub(super) payload_stdio_stdin_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_stdio_stdout_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_stdio_stderr_storage_mode: PayloadStdioStorageMode,
    pub(super) payload_socket_enabled: bool,
    pub(super) payload_socket_redaction_policy: PayloadRedactionPolicy,
    pub(super) payload_socket_retention_max_bytes_per_trace: u64,
    pub(super) socket_payload_gate: SocketHttpPayloadGate,
    pub(super) payload_body_retention_gate: PayloadBodyRetentionGate,
    pub(super) seccomp_notify: SeccompNotifyService,
    pub(super) seccomp_tls: SeccompTlsService,
    pub(super) tls_sync: TlsSyncService,
    pub(super) seccomp_socket: SeccompSocketService,
    pub(super) process_seccomp: ProcessSeccompService,
    pub(super) pending_process_seccomp_observations: Vec<ProcessSeccompObservation>,
    pub(super) application_protocol: ApplicationProtocolAnalyzer,
    pub(super) resource_metrics: ResourceMetricsSampler,
    pub(super) enforcement: FanotifyEnforcementService,
    pub(super) semantic_actions: LiveSemanticActionRuntime,
    pub(super) export_runtime: ExportRuntime,
    pub(super) finalized_terminal_traces: BTreeSet<model_core::ids::TraceId>,
    pub(super) diagnosed_terminal_open_memberships:
        BTreeSet<(model_core::ids::TraceId, ProcessIdentity)>,
    pub(super) provider_classifier: Box<dyn ProviderClassifier>,
    pub(super) provider_classification_enabled: bool,
}

impl StorageAttachService {
    pub(crate) fn collector_name(&self) -> String {
        self.collector.descriptor().name.to_string()
    }

    pub(crate) fn collector_ready(&self) -> bool {
        self.collector.probe_result().reason_unavailable.is_none()
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
    ) -> Result<(model_core::ids::TraceId, ProcessIdentity, DiagnosticKind), ControlError> {
        let root_identity =
            process_identity_contract::lookup::ProcessIdentityReader::read_identity(
                &self.identity_reader,
                command.root_pid,
            )
            .map_err(|error| ControlError::new("identity_lookup", format!("{:?}", error)))?;
        let snapshot = process_tree_snapshot_contract::snapshot::ProcessTreeSnapshotter::snapshot(
            &self.snapshotter,
            &root_identity,
        )
        .map_err(|error| ControlError::new("snapshot", error))?;

        let trace_id = trace_runtime.reserve_trace_id();
        trace_runtime
            .create_starting_trace(
                trace_id,
                TrackTraceRequest {
                    root_identity: root_identity.clone(),
                    display_name: command.display_name.clone(),
                    profile_snapshot,
                    tags: command.tags.clone(),
                    created_at: SystemTime::now(),
                },
                sensor_plan,
            )
            .map_err(|error| ControlError::new("create_trace", format!("{:?}", error)))?;

        let merge_result = merge_snapshot(trace_id, &root_identity, &snapshot, &[]);
        for membership in merge_result.memberships {
            trace_runtime
                .insert_membership(trace_id, membership)
                .map_err(|error| ControlError::new("insert_membership", format!("{:?}", error)))?;
        }
        Ok((
            trace_id,
            root_identity,
            if merge_result.bootstrap_partial {
                DiagnosticKind::BootstrapPartial
            } else {
                DiagnosticKind::BootstrapGap
            },
        ))
    }

    fn finalize_trace(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        trace_id: model_core::ids::TraceId,
        root_identity: ProcessIdentity,
        launch_mode: bool,
        diagnostic_kind: DiagnosticKind,
        diagnostic_message: &'static str,
    ) -> Result<TrackAddReply, ControlError> {
        let emit_bootstrap_diagnostic =
            !launch_mode || matches!(diagnostic_kind, DiagnosticKind::BootstrapPartial);
        if emit_bootstrap_diagnostic {
            trace_runtime
                .mark_degraded(trace_id)
                .map_err(|error| ControlError::new("mark_degraded", format!("{:?}", error)))?;
        }
        trace_runtime
            .activate_trace(trace_id, SystemTime::now())
            .map_err(|error| ControlError::new("activate_trace", format!("{:?}", error)))?;

        let entry = trace_runtime.get_trace(trace_id).ok_or_else(|| {
            ControlError::new("trace_missing", "trace disappeared after activation")
        })?;
        let trace = entry.trace.clone();
        let trace_state = TraceStateRecord::new(
            trace.clone(),
            entry.memberships.memberships().cloned().collect(),
        );
        RecordingWriter::new(self.storage.as_mut())
            .persist_trace_state(trace_state)
            .map_err(recording_error_to_control)?;
        if emit_bootstrap_diagnostic {
            let diagnostic = DiagnosticRecord::new(
                self.next_diagnostic_id()?,
                Some(trace_id),
                diagnostic_kind,
                DiagnosticSeverity::Warning,
                SystemTime::now(),
                diagnostic_message,
            )
            .with_process(root_identity);
            RecordingWriter::new(self.storage.as_mut())
                .persist_diagnostic(diagnostic)
                .map_err(recording_error_to_control)?;
        }
        if launch_mode {
            self.log_diagnostic(
                DiagnosticLogLevel::Info,
                format_args!(
                    "agent_launch started trace_id={} name={} pid={} generation={}",
                    trace_id,
                    trace.display_name,
                    trace.root_process_identity.pid,
                    trace.root_process_identity.generation
                ),
            );
        }

        Ok(TrackAddReply {
            trace_id,
            lifecycle_state: trace.lifecycle_state,
        })
    }

    fn attach_snapshot_only(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        profile_snapshot: CaptureProfileSnapshot,
        sensor_plan: SensorPlan,
    ) -> Result<TrackAddReply, ControlError> {
        let (trace_id, root_identity, diagnostic_kind) =
            self.bootstrap_snapshot(trace_runtime, command, profile_snapshot, sensor_plan)?;
        self.finalize_trace(
            trace_runtime,
            trace_id,
            root_identity,
            command.launch_mode,
            diagnostic_kind,
            "snapshot-only attach completed without eBPF coverage guard",
        )
    }

    fn attach_with_collector(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
        profile_snapshot: CaptureProfileSnapshot,
        sensor_plan: SensorPlan,
    ) -> Result<TrackAddReply, ControlError> {
        let collector_name = self.collector_name();
        let requested_capabilities = collector_capability_requests(
            &profile_snapshot.capability_requests,
            &sensor_plan,
            &collector_name,
        );
        let uses_ebpf_collector = !requested_capabilities.is_empty();
        let (trace_id, root_identity, diagnostic_kind) = self.bootstrap_snapshot(
            trace_runtime,
            command,
            profile_snapshot.clone(),
            sensor_plan,
        )?;

        let member_identities = trace_runtime
            .get_trace(trace_id)
            .ok_or_else(|| {
                ControlError::new("trace_missing", "trace disappeared during bootstrap")
            })?
            .memberships
            .memberships()
            .map(|membership| membership.identity.clone())
            .collect::<Vec<_>>();

        if uses_ebpf_collector {
            if let Err(error) = self.collector.bind_trace(&TraceBindingRequest {
                trace_id,
                root_identity: root_identity.clone(),
                profile_snapshot: profile_snapshot.clone(),
                requested_capabilities,
            }) {
                let _ = trace_runtime.fail_trace(trace_id, SystemTime::now());
                return Err(ControlError::new(error.stage, error.message));
            }

            if let Err(error) = self
                .collector
                .seed_trace_memberships(trace_id, member_identities)
            {
                let _ = trace_runtime.fail_trace(trace_id, SystemTime::now());
                return Err(ControlError::new(error.stage, error.message));
            }
        }

        self.finalize_trace(
            trace_runtime,
            trace_id,
            root_identity,
            command.launch_mode,
            diagnostic_kind,
            if uses_ebpf_collector {
                "snapshot bootstrap completed before live eBPF tracking and remains gap-marked"
            } else {
                "snapshot bootstrap completed before virtual collector sampling and remains gap-marked"
            },
        )
    }
}

impl AttachService for StorageAttachService {
    fn attach_existing(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: &TrackAddCommand,
    ) -> Result<TrackAddReply, ControlError> {
        let profile = self
            .profiles
            .capture_profile(&command.profile_name)
            .ok_or_else(|| {
                ControlError::new("unknown_profile", "capture profile does not exist")
            })?;
        let captured_at = SystemTime::now();
        let profile_snapshot = CaptureProfileSnapshot::from_profile(profile, captured_at);
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
        if self.process_seccomp.enabled()
            && !command.launch_mode
            && capability_requested(
                &profile_snapshot.capability_requests,
                &Capability::ProcExecContext,
            )
        {
            return Err(ControlError::new(
                "process_seccomp_backend",
                "process exec context capture is only supported by actrailctl launch",
            ));
        }
        let sensor_plan = trace_runtime
            .negotiate(&profile_snapshot)
            .map_err(|error| ControlError::new("negotiate", format!("{:?}", error)))?;

        if sensor_plan.collectors.is_empty() {
            return self.attach_snapshot_only(
                trace_runtime,
                command,
                profile_snapshot,
                sensor_plan,
            );
        }

        self.attach_with_collector(trace_runtime, command, profile_snapshot, sensor_plan)
    }

    fn drain_live_events(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        self.drain_live_events_impl(trace_runtime)
    }

    fn event_poll_fds(&self) -> Result<Vec<RawFd>, ControlError> {
        let mut fds = Vec::new();
        if let Some(fd) = self
            .collector
            .event_poll_fd()
            .map_err(|error| ControlError::new(error.stage, error.message))?
        {
            fds.push(fd);
        }
        if let Some(fd) = self.enforcement.event_poll_fd() {
            fds.push(fd);
        }
        fds.extend(self.tls_sync.event_poll_fds());
        fds.extend(self.seccomp_notify.event_poll_fds());
        Ok(fds)
    }

    fn background_poll_timeout(&self) -> Result<Option<Duration>, ControlError> {
        Ok(self.resource_metrics.poll_timeout())
    }

    fn remove_root(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        trace_id: model_core::ids::TraceId,
        removed_at: SystemTime,
    ) -> Result<(), ControlError> {
        self.remove_root_impl(trace_runtime, trace_id, removed_at)
    }

    fn register_seccomp_listener(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
        command: control_contract::command::RegisterSeccompListenerCommand,
    ) -> Result<(), ControlError> {
        let trace = trace_runtime
            .get_trace(command.trace_id)
            .ok_or_else(|| ControlError::new("seccomp_listener", "trace not found"))?;
        let target_known = trace.memberships.memberships().any(|membership| {
            membership.identity.pid == command.target_pid
                || membership
                    .inherited_from
                    .as_ref()
                    .is_some_and(|parent| parent.pid == command.target_pid)
        });
        if !target_known {
            let inherited = self.process_seccomp.ensure_listener_target(
                trace_runtime,
                &self.identity_reader,
                command.trace_id,
                command.target_pid,
            )?;
            if let Some(identity) = inherited {
                if self.collector.stats().active_bindings > 0 {
                    self.collector
                        .seed_trace_memberships(command.trace_id, std::iter::once(identity))
                        .map_err(|error| ControlError::new(error.stage, error.message))?;
                }
                self.persist_trace_state(trace_runtime, command.trace_id)?;
            }
        }
        self.seccomp_notify.register_listener(command.listener_fd)
    }
}

fn recording_error_to_control(error: recording_runtime::RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}
