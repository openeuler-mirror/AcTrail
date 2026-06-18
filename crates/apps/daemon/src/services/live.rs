//! Live collector draining, runtime mutation, and SQLite persistence.

#[path = "live/batch.rs"]
mod batch;
#[path = "live/reconcile.rs"]
mod reconcile;
#[path = "live/shutdown.rs"]
mod shutdown;
#[path = "live/tls_debug.rs"]
mod tls_debug;

use collector_instance::CollectorInstance;
use config_core::daemon::DiagnosticLogLevel;
use control_contract::reply::ControlError;
use model_core::event::{DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload};
use model_core::ids::{CollectorName, DiagnosticId, EventId, TraceId};
use model_core::process::ProcessMembership;
use model_core::trace::TraceLifecycleState;
use recording_runtime::{RecordingWriter, TraceStateRecord};
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::resource_metrics::COLLECTOR_NAME as RESOURCE_METRICS_COLLECTOR_NAME;
use crate::services::workload_diagnostics::PayloadSegmentStage;

impl StorageAttachService {
    pub(super) fn drain_live_events_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        self.drain_resource_metrics_impl(trace_runtime)?;
        self.drain_tls_sync_events_impl(trace_runtime)?;
        let stats = self.collector.stats();
        let active_bindings = stats.active_bindings;
        let active_path = self.collector_ready() && active_bindings > 0;
        self.workload_diagnostics
            .record_drain_call(active_bindings, active_path);
        if !active_path {
            self.drain_seccomp_notifications_impl(trace_runtime)?;
            self.collector
                .poll_tls_payload_control_events()
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            self.log_tls_diagnostic_events_impl();
            self.ingest_polled_seccomp_tls_controls_impl()?;
            self.drain_seccomp_notifications_impl(trace_runtime)?;
            self.flush_process_seccomp_observations_impl(trace_runtime)?;
            self.persist_completed_seccomp_tls_operations_impl(trace_runtime)?;
            self.persist_completed_seccomp_socket_operations_impl(trace_runtime)?;
            self.log_payload_tls_diagnostics_impl()?;
            self.drain_enforcement_impl(trace_runtime)?;
            self.reconcile_draining_memberships_impl(trace_runtime)?;
            self.finalize_terminal_traces_impl(trace_runtime)?;
            self.forget_terminal_trace_state_impl(trace_runtime);
            return Ok(());
        }

        self.drain_seccomp_notifications_impl(trace_runtime)?;
        let batch = self
            .collector
            .poll_batch()
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        self.workload_diagnostics
            .record_collector_batch(batch.observations.len(), batch.payload_segments.len());
        self.log_tls_diagnostic_events_impl();
        self.process_live_event_batch(trace_runtime, batch.observations)?;
        self.process_payload_segments_impl(trace_runtime, batch.payload_segments)?;
        self.ingest_polled_seccomp_tls_controls_impl()?;
        self.drain_seccomp_notifications_impl(trace_runtime)?;
        self.flush_process_seccomp_observations_if_idle(trace_runtime)?;
        self.persist_completed_seccomp_tls_operations_impl(trace_runtime)?;
        self.persist_completed_seccomp_socket_operations_impl(trace_runtime)?;
        self.log_payload_tls_diagnostics_impl()?;
        self.drain_enforcement_impl(trace_runtime)?;
        self.reconcile_draining_memberships_impl(trace_runtime)?;
        self.finalize_terminal_traces_impl(trace_runtime)?;
        self.forget_terminal_trace_state_impl(trace_runtime);
        Ok(())
    }

    fn forget_terminal_trace_state_impl(&mut self, trace_runtime: &TraceRuntime) {
        for trace in trace_runtime.list_trace_records() {
            if matches!(
                trace.lifecycle_state,
                TraceLifecycleState::Completed | TraceLifecycleState::Failed
            ) && self.finalized_terminal_traces.contains(&trace.trace_id)
            {
                self.semantic_actions.forget_trace(trace.trace_id);
                self.application_protocol.forget_trace(trace.trace_id);
                self.payload_body_retention_gate
                    .forget_trace(trace.trace_id);
                self.retained_payload_bytes_by_trace.remove(&trace.trace_id);
            }
        }
    }

    fn ingest_polled_seccomp_tls_controls_impl(&mut self) -> Result<(), ControlError> {
        let direct_captures = self.collector.take_tls_direct_captures();
        let capture_requests = self.collector.take_tls_capture_requests();
        let completions = self.collector.take_tls_completions();
        if self.diagnostic_log_enabled(DiagnosticLogLevel::Debug)
            && (!direct_captures.is_empty()
                || !capture_requests.is_empty()
                || !completions.is_empty())
        {
            self.log_diagnostic(
                DiagnosticLogLevel::Debug,
                format_args!(
                    "tls_payload_ring direct_captures={} capture_requests={} completions={}",
                    direct_captures.len(),
                    capture_requests.len(),
                    completions.len()
                ),
            );
        }
        self.seccomp_tls.ingest_direct_captures(direct_captures)?;
        self.seccomp_tls.ingest_capture_requests(capture_requests)?;
        self.seccomp_tls.ingest_completions(completions)
    }

    fn drain_tls_sync_events_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        let payload_segments = self.tls_sync.drain()?;
        self.workload_diagnostics
            .record_payload_segments(PayloadSegmentStage::TlsSync, payload_segments.len());
        self.process_payload_segments_impl(trace_runtime, payload_segments)
    }

    fn persist_completed_seccomp_tls_operations_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        let payload_segments = self
            .seccomp_tls
            .complete_operations(&self.identity_reader)?;
        self.workload_diagnostics
            .record_payload_segments(PayloadSegmentStage::SeccompTls, payload_segments.len());
        self.process_payload_segments_impl(trace_runtime, payload_segments)
    }

    fn persist_completed_seccomp_socket_operations_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        let completions = self.collector.take_socket_completions();
        let payload_segments = self.seccomp_socket.complete_operations(completions)?;
        self.workload_diagnostics
            .record_payload_segments(PayloadSegmentStage::SeccompSocket, payload_segments.len());
        self.process_payload_segments_impl(trace_runtime, payload_segments)
    }

    fn log_payload_tls_diagnostics_impl(&mut self) -> Result<(), ControlError> {
        if !self.diagnostic_log_enabled(DiagnosticLogLevel::Debug) {
            return Ok(());
        }
        let Some(snapshot) = self
            .collector
            .tls_payload_diagnostics()
            .map_err(|error| ControlError::new(error.stage, error.message))?
        else {
            return Ok(());
        };
        let summary = snapshot.nonzero_summary();
        if self.last_payload_tls_diagnostics.as_deref() == Some(summary.as_str()) {
            return Ok(());
        }
        self.log_diagnostic(
            DiagnosticLogLevel::Debug,
            format_args!("payload_tls_diagnostics {summary}"),
        );
        self.last_payload_tls_diagnostics = Some(summary);
        Ok(())
    }

    fn drain_seccomp_notifications_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        let seccomp_notify = &mut self.seccomp_notify;
        let seccomp_tls = &mut self.seccomp_tls;
        let seccomp_socket = &mut self.seccomp_socket;
        let tls_sync = &self.tls_sync;
        let collector = &mut self.collector;
        let mut process_observations = Vec::new();
        {
            let process_seccomp = &self.process_seccomp;
            let identity_reader = &self.identity_reader;
            seccomp_notify.drain_notifications(|notification, continuation| {
                process_observations.extend(process_seccomp.handle_notification(
                    trace_runtime,
                    identity_reader,
                    notification,
                    continuation,
                    &mut |candidate| {
                        if candidate.path_truncated {
                            return Ok(());
                        }
                        let Some(path) = candidate.path.as_deref() else {
                            return Ok(());
                        };
                        let Some(host_path) = crate::services::process_seccomp::host_exec_path(
                            candidate.pid,
                            path,
                            candidate.execveat_dirfd,
                        ) else {
                            return Ok(());
                        };
                        if let Err(error) = tls_sync.prewarm_plan_for_exec(&host_path) {
                            tracing::warn!(
                                target: "actrail::tls_sync",
                                binary = %host_path.display(),
                                error = %error.message,
                                "failed to prewarm TLS sync plan for exec candidate"
                            );
                        }
                        collector
                            .attach_dynamic_go_tls(&host_path)
                            .map_err(|error| ControlError::new(error.stage, error.message))
                    },
                )?);
                let tls_consumed = seccomp_tls.handle_notification(collector, notification)?;
                if !tls_consumed {
                    seccomp_socket.handle_notification(collector, trace_runtime, notification)?;
                }
                Ok(())
            })?;
        }
        self.pending_process_seccomp_observations
            .extend(process_observations);
        self.process_seccomp
            .ensure_pending_observation_capacity(self.pending_process_seccomp_observations.len())?;
        Ok(())
    }

    fn flush_process_seccomp_observations_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        if self.pending_process_seccomp_observations.is_empty() {
            return Ok(());
        }
        let observations = std::mem::take(&mut self.pending_process_seccomp_observations);
        let raw_events = observations
            .into_iter()
            .map(|observation| {
                self.process_seccomp
                    .materialize_observation(trace_runtime, observation)
            })
            .collect();
        self.process_live_event_batch(trace_runtime, raw_events)
    }

    fn flush_process_seccomp_observations_if_idle(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        if self.seccomp_notify.has_listeners() {
            return Ok(());
        }
        self.flush_process_seccomp_observations_impl(trace_runtime)
    }

    fn drain_resource_metrics_impl(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let mut events = Vec::new();
        for draft in self.resource_metrics.drain_due(trace_runtime)? {
            let event = DomainEvent::new(
                EventEnvelope {
                    event_id: self.next_event_id()?,
                    trace_id: draft.trace_id,
                    observed_at: draft.observed_at,
                    process: draft.process,
                    collector: CollectorName::new(RESOURCE_METRICS_COLLECTOR_NAME),
                    kind: EventKind::Resource,
                    flags: EventFlags::clean(),
                },
                EventPayload::Resource(draft.payload),
            );
            events.push(event);
        }
        self.persist_observed_event_batch(trace_runtime, events)
    }

    fn drain_enforcement_impl(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let mut events = Vec::new();
        for draft in self
            .enforcement
            .drain_due(trace_runtime, &self.identity_reader)?
        {
            let event = DomainEvent::new(
                EventEnvelope {
                    event_id: self.next_event_id()?,
                    trace_id: draft.trace_id,
                    observed_at: draft.observed_at,
                    process: draft.process,
                    collector: CollectorName::new(crate::services::enforcement::COLLECTOR_NAME),
                    kind: EventKind::Enforcement,
                    flags: EventFlags {
                        metadata_partial: draft.metadata_partial,
                        ..EventFlags::clean()
                    },
                },
                EventPayload::Enforcement(draft.payload),
            );
            events.push(event);
        }
        self.persist_observed_event_batch(trace_runtime, events)
    }

    pub(super) fn next_diagnostic_id(&mut self) -> Result<DiagnosticId, ControlError> {
        next_diagnostic_id_from_seed(&mut self.next_diagnostic_id)
    }

    pub(super) fn next_event_id(&mut self) -> Result<EventId, ControlError> {
        let raw = self.next_event_id;
        self.next_event_id = self
            .next_event_id
            .checked_add(1)
            .ok_or_else(|| ControlError::new("event_id_overflow", "event id overflow"))?;
        Ok(EventId::new(raw))
    }

    pub(super) fn persist_trace_state(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
    ) -> Result<(), ControlError> {
        let terminal = trace_runtime
            .get_trace(trace_id)
            .map(|entry| {
                matches!(
                    entry.trace.lifecycle_state,
                    TraceLifecycleState::Completed | TraceLifecycleState::Failed
                )
            })
            .ok_or_else(|| ControlError::new("persist_trace_state", "trace not found"))?;
        let trace_state = self.trace_state_record_for_persistence(trace_runtime, trace_id)?;
        RecordingWriter::new(self.storage.as_mut())
            .persist_trace_state(trace_state)
            .map_err(recording_error_to_control)?;

        if terminal {
            self.collector
                .unbind_trace(trace_id)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }

        Ok(())
    }

    pub(in crate::services) fn trace_state_record_for_persistence(
        &self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
    ) -> Result<TraceStateRecord, ControlError> {
        trace_runtime
            .get_trace(trace_id)
            .map(|entry| {
                TraceStateRecord::new(
                    entry.trace.clone(),
                    entry
                        .memberships
                        .memberships()
                        .cloned()
                        .collect::<Vec<ProcessMembership>>(),
                )
            })
            .ok_or_else(|| ControlError::new("persist_trace_state", "trace not found"))
    }
}

pub(super) fn next_diagnostic_id_from_seed(seed: &mut u64) -> Result<DiagnosticId, ControlError> {
    let raw = *seed;
    *seed = seed
        .checked_add(1)
        .ok_or_else(|| ControlError::new("diagnostic_id_overflow", "diagnostic id overflow"))?;
    Ok(DiagnosticId::new(raw))
}

fn recording_error_to_control(error: recording_runtime::RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}
