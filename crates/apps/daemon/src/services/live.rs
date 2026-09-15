//! Live collector draining, runtime mutation, and SQLite persistence.

#[path = "live/batch.rs"]
mod batch;
#[path = "live/launch_binding.rs"]
mod launch_binding;
#[path = "live/llm_diagnostics.rs"]
mod llm_diagnostics;
#[path = "live/mcp_diagnostics.rs"]
mod mcp_diagnostics;
#[path = "live/reconcile.rs"]
mod reconcile;
#[path = "live/seccomp.rs"]
mod seccomp;
#[path = "live/shutdown.rs"]
mod shutdown;
#[path = "live/tls_debug.rs"]
mod tls_debug;

use std::collections::BTreeSet;
use std::time::SystemTime;

use collector_instance::CollectorInstance;
use config_core::daemon::DiagnosticLogLevel;
use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::event::{DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload};
use model_core::ids::{CollectorName, DiagnosticId, EventId, TraceId};
use model_core::process::{ProcessIdentity, ProcessMembership};
use model_core::resource_scope::ResourceScopeLifecycleState;
use model_core::trace::{TraceHealth, TraceLifecycleState};
use recording_runtime::{RecordingWriter, SemanticActionBatch, TraceStateRecord};
use trace_runtime::membership::MembershipIndex;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::command_control::CommandEnforcementDraft;
use crate::services::resource_metrics::COLLECTOR_NAME as RESOURCE_METRICS_COLLECTOR_NAME;
use crate::services::workload_diagnostics::PayloadSegmentStage;

/// Log and swallow recoverable errors from best-effort subsystems.
///
/// On the hot path this is a single `if let Err` branch — zero heap, zero
/// string comparisons. The CPU predictor gets it right 99.999% of the time.
#[inline]
fn warn_best_effort(result: Result<(), ControlError>, label: &str) {
    if let Err(error) = result {
        tracing::warn!(
            %label,
            error.code = %error.code,
            error.message = %error.message,
            "best-effort observation subsystem error; drain cycle continues"
        );
    }
}

impl StorageAttachService {
    pub(super) fn drain_live_events_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        self.drain_alert_ingress_impl()?;
        self.tick_idle_detector_impl()?;
        self.drain_post_trace_runtime_impl()?;
        self.drain_resource_metrics_impl(trace_runtime)?;
        self.drain_tls_sync_events_impl(trace_runtime)?;
        let active_bindings = self.collector.active_binding_trace_count();
        let active_path = self.collector_ready() && active_bindings > 0;
        self.workload_diagnostics
            .record_drain_call(active_bindings, active_path);
        if !active_path {
            self.drain_seccomp_notifications_impl(trace_runtime)?;
            self.materialize_process_seccomp_observations_impl(trace_runtime)?;
            let poll_result = self
                .collector
                .poll_tls_payload_control_events()
                .map_err(|error| ControlError::new(error.stage, error.message));
            warn_best_effort(
                self.ingest_polled_seccomp_tls_controls_impl(),
                "seccomp_tls_control",
            );
            poll_result?;
            warn_best_effort(
                self.persist_launch_binding_failures_impl(trace_runtime),
                "launch_binding_failure",
            );
            warn_best_effort(
                self.persist_event_transport_loss_diagnostics_impl(trace_runtime),
                "event_transport_loss_diag",
            );
            self.log_tls_diagnostic_events_impl();
            self.drain_seccomp_notifications_impl(trace_runtime)?;
            self.materialize_process_seccomp_observations_impl(trace_runtime)?;
            self.persist_completed_seccomp_tls_operations_impl(trace_runtime)?;
            self.persist_completed_seccomp_socket_operations_impl(trace_runtime)?;
            warn_best_effort(self.log_payload_tls_diagnostics_impl(), "payload_tls_diag");
            warn_best_effort(self.drain_enforcement_impl(trace_runtime), "enforcement");
            let mcp_stdio_diagnostics = self
                .semantic_actions
                .flush_closed_mcp_stdio_sessions_with_diagnostics(SystemTime::now());
            self.persist_mcp_stdio_diagnostics_impl(trace_runtime, mcp_stdio_diagnostics)?;
            self.reconcile_draining_memberships_impl(trace_runtime)?;
            self.finalize_terminal_traces_impl(trace_runtime)?;
            self.forget_terminal_trace_state_impl(trace_runtime);
            self.sweep_storage_retention_impl(trace_runtime)?;
            let _ = self.collector.flush_transport();
            return Ok(());
        }

        self.drain_seccomp_notifications_impl(trace_runtime)?;
        self.materialize_process_seccomp_observations_impl(trace_runtime)?;
        let drain_probe_started = std::time::Instant::now();
        let drain_probe_poll = std::time::Instant::now();
        let batch_result = self
            .collector
            .poll_batch()
            .map_err(|error| ControlError::new(error.stage, error.message));
        let drain_probe_poll_ms = drain_probe_poll.elapsed().as_millis();
        warn_best_effort(
            self.ingest_polled_seccomp_tls_controls_impl(),
            "seccomp_tls_control",
        );
        let batch = batch_result?;
        let observations_count = batch.observations.len();
        let payload_segments_count = batch.payload_segments.len();
        let payload_stream_closes = batch.payload_stream_closes;
        warn_best_effort(
            self.persist_launch_binding_failures_impl(trace_runtime),
            "launch_binding_failure",
        );
        warn_best_effort(
            self.persist_event_transport_loss_diagnostics_impl(trace_runtime),
            "event_transport_loss_diag",
        );
        self.workload_diagnostics
            .record_collector_batch(batch.observations.len(), batch.payload_segments.len());
        self.log_tls_diagnostic_events_impl();
        let drain_probe_events = std::time::Instant::now();
        self.process_live_event_batch(trace_runtime, batch.observations)?;
        let drain_probe_events_ms = drain_probe_events.elapsed().as_millis();
        let drain_probe_payloads = std::time::Instant::now();
        self.process_payload_segments_impl(trace_runtime, batch.payload_segments)?;
        self.process_payload_stream_closes_impl(trace_runtime, payload_stream_closes)?;
        let drain_probe_payloads_ms = drain_probe_payloads.elapsed().as_millis();
        let mcp_stdio_diagnostics = self
            .semantic_actions
            .flush_closed_mcp_stdio_sessions_with_diagnostics(SystemTime::now());
        self.persist_mcp_stdio_diagnostics_impl(trace_runtime, mcp_stdio_diagnostics)?;
        self.drain_seccomp_notifications_impl(trace_runtime)?;
        self.materialize_process_seccomp_observations_impl(trace_runtime)?;
        self.persist_completed_seccomp_tls_operations_impl(trace_runtime)?;
        self.persist_completed_seccomp_socket_operations_impl(trace_runtime)?;
        warn_best_effort(self.log_payload_tls_diagnostics_impl(), "payload_tls_diag");
        warn_best_effort(self.drain_enforcement_impl(trace_runtime), "enforcement");
        self.reconcile_draining_memberships_impl(trace_runtime)?;
        self.finalize_terminal_traces_impl(trace_runtime)?;
        self.forget_terminal_trace_state_impl(trace_runtime);
        self.sweep_storage_retention_impl(trace_runtime)?;
        let _ = self.collector.flush_transport();
        let drain_probe_total_ms = drain_probe_started.elapsed().as_millis();
        if drain_probe_total_ms >= 5000 {
            tracing::warn!(
                target: "actrail::perfprobe",
                total_ms = drain_probe_total_ms,
                poll_ms = drain_probe_poll_ms,
                events_ms = drain_probe_events_ms,
                payloads_ms = drain_probe_payloads_ms,
                observations = observations_count,
                payload_segments = payload_segments_count,
                "slow drain cycle"
            );
        }
        Ok(())
    }

    fn forget_terminal_trace_state_impl(&mut self, trace_runtime: &TraceRuntime) {
        for trace in trace_runtime.list_trace_records() {
            if trace.lifecycle_state.is_terminal()
                && self.finalized_terminal_traces.contains(&trace.trace_id)
            {
                self.semantic_actions.forget_trace(trace.trace_id);
                self.application_protocol.forget_trace(trace.trace_id);
                self.payload_reorderer.forget_trace(trace.trace_id);
                self.seccomp_socket.forget_trace(trace.trace_id);
                self.socket_payload_gate.forget_trace(trace.trace_id);
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
        let capture_result = self.seccomp_tls.ingest_capture_requests(capture_requests);
        let direct_result = self.seccomp_tls.ingest_direct_captures(direct_captures);
        let completion_result = self.seccomp_tls.ingest_completions(completions);
        capture_result.and(direct_result).and(completion_result)
    }

    fn drain_tls_sync_events_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let drain = match self.tls_sync.drain(trace_runtime) {
            Ok(d) => d,
            Err(error) => {
                tracing::warn!(
                    error.code = %error.code,
                    error.message = %error.message,
                    "TLS sync drain failed; skipping this cycle"
                );
                return Ok(());
            }
        };
        self.workload_diagnostics
            .record_payload_segments(PayloadSegmentStage::TlsSync, drain.payload_segments.len());
        self.persist_tls_sync_diagnostics_impl(trace_runtime, drain.diagnostics)?;
        if !drain.flow_diagnostics.is_empty() {
            // Observability-side write: fail local instead of aborting the
            // whole drain cycle (which would drop the event/payload writes that
            // were already committed to this cycle).
            if let Err(error) = self
                .storage
                .as_mut()
                .append_tls_flow_diagnostics(drain.flow_diagnostics)
            {
                tracing::warn!(
                    stage = %error.stage,
                    message = %error.message,
                    "tls flow diagnostics write failed; continuing drain"
                );
            }
        }
        self.process_payload_segments_impl(trace_runtime, drain.payload_segments)
    }

    fn persist_tls_sync_diagnostics_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
        diagnostics: Vec<crate::services::tls_sync::TlsSyncDiagnostic>,
    ) -> Result<(), ControlError> {
        let drafts = diagnostics
            .into_iter()
            .map(|diagnostic| RuntimeDropDiagnosticDraft {
                trace_id: None,
                code: diagnostic.code,
                message: diagnostic.message,
            })
            .collect();
        self.persist_runtime_drop_diagnostics(trace_runtime, drafts, Vec::new())
    }

    fn persist_event_transport_loss_diagnostics_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let losses = self.collector.take_event_transport_loss_summaries();
        if losses.is_empty() {
            return Ok(());
        }

        let active_trace_ids = non_terminal_trace_ids(trace_runtime);
        let mut trace_state_ids = BTreeSet::new();
        let mut drafts = Vec::new();
        let loss_message = event_transport_loss_message(&losses);
        if active_trace_ids.is_empty() {
            drafts.push(RuntimeDropDiagnosticDraft {
                trace_id: None,
                code: "event_transport_loss".to_string(),
                message: loss_message,
            });
        } else {
            for trace_id in &active_trace_ids {
                trace_runtime.mark_degraded(*trace_id).map_err(|error| {
                    ControlError::new("event_transport_loss_degrade", format!("{error:?}"))
                })?;
                trace_state_ids.insert(*trace_id);
                drafts.push(RuntimeDropDiagnosticDraft {
                    trace_id: Some(*trace_id),
                    code: "event_transport_loss".to_string(),
                    message: loss_message.clone(),
                });
            }
        }
        let trace_states = trace_state_ids
            .into_iter()
            .map(|trace_id| self.trace_state_record_for_persistence(trace_runtime, trace_id))
            .collect::<Result<Vec<_>, _>>()?;
        self.persist_runtime_drop_diagnostics(trace_runtime, drafts, trace_states)
    }

    fn persist_runtime_drop_diagnostics(
        &mut self,
        trace_runtime: &TraceRuntime,
        drafts: Vec<RuntimeDropDiagnosticDraft>,
        trace_states: Vec<TraceStateRecord>,
    ) -> Result<(), ControlError> {
        if drafts.is_empty() && trace_states.is_empty() {
            return Ok(());
        }
        let emitted_at = SystemTime::now();
        let diagnostics = drafts
            .into_iter()
            .map(|draft| {
                Ok(DiagnosticRecord::new(
                    self.next_diagnostic_id()?,
                    draft.trace_id,
                    DiagnosticKind::RuntimeDropped,
                    DiagnosticSeverity::Warning,
                    emitted_at,
                    draft.message,
                )
                .with_metadata("code", draft.code))
            })
            .collect::<Result<Vec<_>, ControlError>>()?;
        self.persist_observed_batch_then_publish(
            trace_runtime,
            Vec::new(),
            diagnostics,
            SemanticActionBatch::default(),
            trace_states,
            Vec::new(),
        )
    }

    fn persist_completed_seccomp_tls_operations_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
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
        trace_runtime: &mut TraceRuntime,
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

    fn drain_resource_metrics_impl(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let drain = match self
            .resource_metrics
            .drain_due(trace_runtime, &self.process_registry)
        {
            Ok(d) => d,
            Err(error) => {
                tracing::warn!(
                    error.code = %error.code,
                    error.message = %error.message,
                    "resource metrics sampling failed; skipping this cycle"
                );
                return Ok(());
            }
        };
        let mut events = Vec::new();
        let mut recovered_events = Vec::new();
        for draft in drain.samples {
            let recovered = draft.recovered;
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
            if recovered {
                recovered_events.push(event);
            } else {
                events.push(event);
            }
        }
        self.persist_observed_event_batch(trace_runtime, events)?;
        for event in recovered_events {
            self.storage
                .append_event(event)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        for failure in drain.failures {
            if failure.recovered {
                self.storage
                    .update_trace_health(failure.trace_id, TraceHealth::Degraded)
                    .map_err(|error| ControlError::new(error.stage, error.message))?;
            } else {
                trace_runtime
                    .mark_degraded(failure.trace_id)
                    .map_err(|error| {
                        ControlError::new("resource_metrics_degraded", format!("{error:?}"))
                    })?;
            }
            let diagnostic = DiagnosticRecord::new(
                self.next_diagnostic_id()?,
                Some(failure.trace_id),
                DiagnosticKind::RuntimeFailure,
                DiagnosticSeverity::Warning,
                SystemTime::now(),
                "required cgroup resource counter read failed",
            )
            .with_metadata("error", failure.message);
            RecordingWriter::new(self.storage.as_mut())
                .persist_diagnostic(diagnostic)
                .map_err(recording_error_to_control)?;
            if !failure.recovered {
                self.persist_trace_state(trace_runtime, failure.trace_id)?;
            }
        }
        self.drain_resource_finalizations_impl(trace_runtime)
    }

    fn drain_resource_finalizations_impl(
        &mut self,
        trace_runtime: &mut trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let poll = self
            .resource_metrics
            .poll_finalizations(trace_runtime, &self.process_registry)?;
        for trace_id in poll.waiting {
            self.storage
                .update_resource_scope_state(
                    trace_id,
                    ResourceScopeLifecycleState::WaitingForEmpty,
                    None,
                    SystemTime::now(),
                )
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }

        for finalization in poll.ready {
            let event_id = self.next_event_id()?;
            let event = DomainEvent::new(
                EventEnvelope {
                    event_id,
                    trace_id: finalization.trace_id,
                    observed_at: finalization.sample.observed_at,
                    process: finalization.sample.process,
                    collector: CollectorName::new(RESOURCE_METRICS_COLLECTOR_NAME),
                    kind: EventKind::Resource,
                    flags: EventFlags {
                        metadata_partial: finalization.timed_out,
                        ..EventFlags::clean()
                    },
                },
                EventPayload::Resource(finalization.sample.payload),
            );
            let lifecycle_state = if finalization.timed_out {
                ResourceScopeLifecycleState::Orphaned
            } else {
                ResourceScopeLifecycleState::Finalized
            };
            self.persist_resource_final_event(event, lifecycle_state)?;
            if finalization.recovered {
                self.complete_recovered_resource_trace(
                    finalization.trace_id,
                    finalization.timed_out,
                )?;
                self.resource_metrics
                    .finish_finalization(finalization.trace_id, !finalization.timed_out);
                continue;
            }
            if finalization.timed_out {
                trace_runtime
                    .mark_degraded(finalization.trace_id)
                    .map_err(|error| {
                        ControlError::new("resource_finalization_degraded", format!("{error:?}"))
                    })?;
            }
            let has_open_memberships = trace_runtime
                .complete_after_resource_barrier(finalization.trace_id, SystemTime::now())
                .map_err(|error| {
                    ControlError::new("resource_finalization_transition", format!("{error:?}"))
                })?;
            if has_open_memberships {
                self.persist_resource_open_membership_diagnostic(
                    trace_runtime,
                    finalization.trace_id,
                )?;
            }
            self.persist_trace_state(trace_runtime, finalization.trace_id)?;
            self.resource_metrics
                .finish_finalization(finalization.trace_id, !finalization.timed_out);
        }
        Ok(())
    }

    fn complete_recovered_resource_trace(
        &mut self,
        trace_id: TraceId,
        timed_out: bool,
    ) -> Result<(), ControlError> {
        let completed_at = SystemTime::now();
        let mut trace = self
            .storage
            .get_trace(trace_id)
            .map_err(|error| ControlError::new(error.stage, error.message))?
            .ok_or_else(|| {
                ControlError::new("resource_recovery", "recovered trace record is missing")
            })?;
        trace.lifecycle_state = TraceLifecycleState::Completed;
        trace.timings.completed_at = Some(completed_at);
        if timed_out {
            trace.health = TraceHealth::Degraded;
        }
        self.storage
            .create_trace(trace)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let diagnostic = DiagnosticRecord::new(
            self.next_diagnostic_id()?,
            Some(trace_id),
            DiagnosticKind::RuntimeFailure,
            if timed_out {
                DiagnosticSeverity::Warning
            } else {
                DiagnosticSeverity::Info
            },
            completed_at,
            "resource scope finalized after daemon restart",
        )
        .with_metadata("timed_out", timed_out.to_string());
        RecordingWriter::new(self.storage.as_mut())
            .persist_diagnostic(diagnostic)
            .map_err(recording_error_to_control)
    }

    fn persist_resource_final_event(
        &mut self,
        event: DomainEvent,
        lifecycle_state: ResourceScopeLifecycleState,
    ) -> Result<EventId, ControlError> {
        let trace_id = event.envelope.trace_id;
        if let Some(existing) = self
            .storage
            .get_resource_scope(trace_id)
            .map_err(|error| ControlError::new(error.stage, error.message))?
            .and_then(|scope| scope.final_event_id)
        {
            return Ok(existing);
        }
        let event_id = event.envelope.event_id;
        let transaction = self
            .storage
            .begin()
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let write_result = self.storage.append_event(event).and_then(|()| {
            self.storage.update_resource_scope_state(
                trace_id,
                lifecycle_state,
                Some(event_id),
                SystemTime::now(),
            )
        });
        match write_result {
            Ok(()) => transaction
                .commit()
                .map_err(|error| ControlError::new(error.stage, error.message))?,
            Err(error) => {
                let rollback = transaction.rollback();
                let message = match rollback {
                    Ok(()) => error.message,
                    Err(rollback) => format!(
                        "{}; rollback failed at {}: {}",
                        error.message, rollback.stage, rollback.message
                    ),
                };
                return Err(ControlError::new(error.stage, message));
            }
        }
        Ok(event_id)
    }

    fn persist_resource_open_membership_diagnostic(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
    ) -> Result<(), ControlError> {
        let root = trace_runtime
            .get_trace(trace_id)
            .map(|entry| entry.trace.root_process_identity)
            .ok_or_else(|| ControlError::new("resource_finalization", "trace not found"))?;
        if !self
            .diagnosed_terminal_open_memberships
            .insert((trace_id, root))
        {
            return Ok(());
        }
        let diagnostic = DiagnosticRecord::new(
            self.next_diagnostic_id()?,
            Some(trace_id),
            DiagnosticKind::RuntimeFailure,
            DiagnosticSeverity::Warning,
            SystemTime::now(),
            "resource_finalized_with_open_memberships",
        )
        .with_process(root);
        RecordingWriter::new(self.storage.as_mut())
            .persist_diagnostic(diagnostic)
            .map_err(recording_error_to_control)
    }

    fn drain_enforcement_impl(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let drain = self.enforcement.drain_due(
            trace_runtime,
            &mut self.process_registry,
            &self.identity_reader,
            &self.collector,
            &self.control_plugins,
        )?;
        self.persist_enforcement_outcomes(trace_runtime, drain)
    }

    fn persist_enforcement_outcomes(
        &mut self,
        trace_runtime: &TraceRuntime,
        drain: crate::services::enforcement::EnforcementDrain,
    ) -> Result<(), ControlError> {
        let crate::services::enforcement::EnforcementDrain {
            outcomes,
            process_records,
        } = drain;
        let mut events = Vec::new();
        for outcome in outcomes {
            if let Some(alert) = outcome.boundary_alert {
                let alert_token = trace_runtime
                    .get_trace(outcome.trace_id)
                    .map(|entry| entry.trace.alert_token.clone());
                match alert_token {
                    Some(alert_token) => {
                        if let Err(error) = self.alert_ingress.submit_file_access_boundary_alert(
                            outcome.trace_id,
                            alert_token,
                            alert,
                        ) {
                            tracing::warn!(
                                trace_id = %outcome.trace_id,
                                error.code = %error.code,
                                error.message = %error.message,
                                "file access boundary alert queue admission failed"
                            );
                        }
                    }
                    None => {
                        tracing::warn!(
                            trace_id = %outcome.trace_id,
                            "file access boundary alert lost its trace runtime before queue admission"
                        );
                    }
                }
            }
            let Some(draft) = outcome.audit else {
                continue;
            };
            let event = DomainEvent::new(
                EventEnvelope {
                    event_id: self.next_event_id()?,
                    trace_id: outcome.trace_id,
                    observed_at: outcome.observed_at,
                    process: outcome.process,
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
        self.persist_observed_event_batch_with_process_records(
            trace_runtime,
            events,
            process_records,
        )
    }

    fn persist_command_enforcement_outcomes(
        &mut self,
        trace_runtime: &TraceRuntime,
        outcomes: Vec<CommandEnforcementDraft>,
    ) -> Result<(), ControlError> {
        let mut events = Vec::new();
        for outcome in outcomes {
            if let Some(alert) = outcome.boundary_alert {
                let alert_token = trace_runtime
                    .get_trace(outcome.trace_id)
                    .map(|entry| entry.trace.alert_token.clone());
                match alert_token {
                    Some(alert_token) => {
                        if let Err(error) =
                            self.alert_ingress.submit_command_execution_boundary_alert(
                                outcome.trace_id,
                                alert_token,
                                alert,
                            )
                        {
                            tracing::warn!(
                                trace_id = %outcome.trace_id,
                                error.code = %error.code,
                                error.message = %error.message,
                                "command execution boundary alert queue admission failed"
                            );
                        }
                    }
                    None => tracing::warn!(
                        trace_id = %outcome.trace_id,
                        "command execution boundary alert lost its trace before queue admission"
                    ),
                }
            }
            events.push(DomainEvent::new(
                EventEnvelope {
                    event_id: self.next_event_id()?,
                    trace_id: outcome.trace_id,
                    observed_at: outcome.observed_at,
                    process: outcome.process,
                    collector: CollectorName::new(
                        crate::services::process_seccomp::PROCESS_SECCOMP_COLLECTOR_NAME,
                    ),
                    kind: EventKind::Enforcement,
                    flags: EventFlags {
                        metadata_partial: outcome.metadata_partial,
                        ..EventFlags::clean()
                    },
                },
                EventPayload::Enforcement(outcome.payload),
            ));
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
        let trace_state = self.trace_state_record_for_persistence(trace_runtime, trace_id)?;
        RecordingWriter::new(self.storage.as_mut())
            .persist_trace_state(trace_state)
            .map_err(recording_error_to_control)?;

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

    pub(in crate::services) fn trace_state_record_for_memberships(
        &self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
        membership_ids: &BTreeSet<ProcessIdentity>,
    ) -> Result<TraceStateRecord, ControlError> {
        // Trace-state persistence upserts the supplied memberships and never
        // deletes omitted rows, so hot-path batches only need their mutations.
        // Lifecycle/finalization paths continue to use the full snapshot above.
        let entry = trace_runtime
            .get_trace(trace_id)
            .ok_or_else(|| ControlError::new("persist_trace_state", "trace not found"))?;
        let memberships =
            memberships_for_persistence(trace_id, &entry.memberships, membership_ids)?;
        Ok(TraceStateRecord::new(entry.trace.clone(), memberships))
    }
}

fn memberships_for_persistence(
    trace_id: TraceId,
    memberships: &MembershipIndex,
    membership_ids: &BTreeSet<ProcessIdentity>,
) -> Result<Vec<ProcessMembership>, ControlError> {
    membership_ids
        .iter()
        .map(|identity| {
            memberships.get(identity).cloned().ok_or_else(|| {
                ControlError::new(
                    "persist_trace_membership",
                    format!("trace {trace_id} membership {identity} not found"),
                )
            })
        })
        .collect()
}

pub(super) fn next_diagnostic_id_from_seed(seed: &mut u64) -> Result<DiagnosticId, ControlError> {
    let raw = *seed;
    *seed = seed
        .checked_add(1)
        .ok_or_else(|| ControlError::new("diagnostic_id_overflow", "diagnostic id overflow"))?;
    Ok(DiagnosticId::new(raw))
}

struct RuntimeDropDiagnosticDraft {
    trace_id: Option<TraceId>,
    code: String,
    message: String,
}

fn non_terminal_trace_ids(trace_runtime: &TraceRuntime) -> Vec<TraceId> {
    trace_runtime
        .list_trace_records()
        .into_iter()
        .filter(|trace| !trace.lifecycle_state.is_terminal())
        .map(|trace| trace.trace_id)
        .collect()
}

fn event_transport_loss_message(losses: &[String]) -> String {
    match losses {
        [] => String::new(),
        [loss] => loss.clone(),
        _ => format!(
            "{} kernel event transport loss reports: {}",
            losses.len(),
            losses.join("; ")
        ),
    }
}

fn recording_error_to_control(error: recording_runtime::RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}
