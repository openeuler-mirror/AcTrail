//! Live collector draining, runtime mutation, and SQLite persistence.

#[path = "live/batch.rs"]
mod batch;
#[path = "live/reconcile.rs"]
mod reconcile;
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
use model_core::process::ProcessMembership;
use recording_runtime::{RecordingWriter, SemanticActionBatch, TraceStateRecord};
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::command_control::CommandControlOutcome;
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
            warn_best_effort(
                self.persist_event_transport_loss_diagnostics_impl(trace_runtime),
                "event_transport_loss_diag",
            );
            self.log_tls_diagnostic_events_impl();
            self.ingest_polled_seccomp_tls_controls_impl()?;
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
            return Ok(());
        }

        self.drain_seccomp_notifications_impl(trace_runtime)?;
        let batch = self
            .collector
            .poll_batch()
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        warn_best_effort(
            self.persist_event_transport_loss_diagnostics_impl(trace_runtime),
            "event_transport_loss_diag",
        );
        self.workload_diagnostics
            .record_collector_batch(batch.observations.len(), batch.payload_segments.len());
        self.log_tls_diagnostic_events_impl();
        self.process_live_event_batch(trace_runtime, batch.observations)?;
        self.process_payload_segments_impl(trace_runtime, batch.payload_segments)?;
        self.ingest_polled_seccomp_tls_controls_impl()?;
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
        Ok(())
    }

    fn forget_terminal_trace_state_impl(&mut self, trace_runtime: &TraceRuntime) {
        for trace in trace_runtime.list_trace_records() {
            if trace.lifecycle_state.is_terminal()
                && self.finalized_terminal_traces.contains(&trace.trace_id)
            {
                self.semantic_actions.forget_trace(trace.trace_id);
                self.application_protocol.forget_trace(trace.trace_id);
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
        self.seccomp_tls.ingest_direct_captures(direct_captures)?;
        self.seccomp_tls.ingest_capture_requests(capture_requests)?;
        self.seccomp_tls.ingest_completions(completions)
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

    fn drain_seccomp_notifications_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let seccomp_notify = &mut self.seccomp_notify;
        let seccomp_tls = &mut self.seccomp_tls;
        let seccomp_socket = &mut self.seccomp_socket;
        let tls_sync = &self.tls_sync;
        let collector = &mut self.collector;
        let mut network_events = Vec::new();
        let pending_process_observations = &mut self.pending_process_seccomp_observations;
        {
            let process_seccomp = &self.process_seccomp;
            let command_control = &self.command_control;
            let network_control = &self.network_control;
            let control_plugins = &self.control_plugins;
            let identity_reader = &self.identity_reader;
            seccomp_notify.drain_notifications(|notification, continuation| {
                network_events.extend(network_control.handle_notification(
                    trace_runtime,
                    &self.process_registry,
                    identity_reader,
                    notification,
                    continuation,
                    control_plugins,
                )?);
                pending_process_observations.extend(process_seccomp.handle_notification(
                    trace_runtime,
                    &self.process_registry,
                    identity_reader,
                    notification,
                    continuation,
                    &mut |candidate, continuation| {
                        if let Some(trace_id) = candidate.trace_id {
                            match command_control.decide_exec(
                                trace_id,
                                &candidate.process,
                                &self.process_registry,
                                candidate,
                                control_plugins,
                            )? {
                                CommandControlOutcome::Continue => {}
                                outcome => {
                                    let metadata = command_control_metadata(&outcome);
                                    continuation.set_metadata(metadata);
                                    if matches!(
                                        command_control_decision(&outcome),
                                        config_core::daemon::EnforcementDecision::Deny
                                    ) {
                                        continuation.deny_errno(libc::EPERM)?;
                                    } else {
                                        continuation.continue_now()?;
                                    }
                                    return Ok(());
                                }
                            }
                        }
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
                    seccomp_socket.handle_notification(
                        collector,
                        trace_runtime,
                        &self.process_registry,
                        notification,
                    )?;
                }
                Ok(())
            })?;
        }
        self.process_live_event_batch(trace_runtime, network_events)?;
        Ok(())
    }

    fn materialize_process_seccomp_observations_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        if self.pending_process_seccomp_observations.is_empty() {
            return Ok(());
        }
        let batch_size = self.process_seccomp.pending_observation_batch_size()?;
        while !self.pending_process_seccomp_observations.is_empty() {
            let batch_len = self
                .pending_process_seccomp_observations
                .len()
                .min(batch_size);
            let raw_events = self.pending_process_seccomp_observations[..batch_len]
                .iter()
                .map(|observation| {
                    self.process_seccomp.materialize_observation(
                        trace_runtime,
                        &self.process_registry,
                        observation,
                    )
                })
                .collect();
            self.process_live_event_batch(trace_runtime, raw_events)?;
            self.pending_process_seccomp_observations.drain(..batch_len);
        }
        Ok(())
    }

    fn drain_resource_metrics_impl(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
        let drafts = match self
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
        for draft in drafts {
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
        for draft in self.enforcement.drain_due(
            trace_runtime,
            &self.process_registry,
            &self.identity_reader,
            &self.control_plugins,
        )? {
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

fn command_control_metadata(
    outcome: &CommandControlOutcome,
) -> std::collections::BTreeMap<String, String> {
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("subject".to_string(), "command-execution".to_string());
    metadata.insert("decision_source".to_string(), "sync-plugin".to_string());
    metadata.insert(
        "decision".to_string(),
        command_control_decision(outcome).as_str().to_string(),
    );
    match outcome {
        CommandControlOutcome::Continue => {}
        CommandControlOutcome::Decision {
            rule_id,
            plugin_instance,
            timeout_ms,
            concurrency_limit,
            ..
        }
        | CommandControlOutcome::DecisionError {
            rule_id,
            plugin_instance,
            timeout_ms,
            concurrency_limit,
            ..
        } => {
            metadata.insert("rule_id".to_string(), rule_id.clone());
            metadata.insert("plugin_instance".to_string(), plugin_instance.clone());
            metadata.insert("plugin_timeout_ms".to_string(), timeout_ms.to_string());
            metadata.insert(
                "plugin_concurrency_limit".to_string(),
                concurrency_limit.to_string(),
            );
        }
    }
    if let CommandControlOutcome::DecisionError { error, .. } = outcome {
        metadata.insert("plugin_error".to_string(), error.clone());
        let fallback_reason = if error == "concurrency_limit" {
            "concurrency_limit"
        } else {
            "plugin_error"
        };
        metadata.insert("fallback_reason".to_string(), fallback_reason.to_string());
    }
    metadata
}

fn command_control_decision(
    outcome: &CommandControlOutcome,
) -> config_core::daemon::EnforcementDecision {
    match outcome {
        CommandControlOutcome::Continue => config_core::daemon::EnforcementDecision::Allow,
        CommandControlOutcome::Decision { decision, .. }
        | CommandControlOutcome::DecisionError { decision, .. } => *decision,
    }
}

fn recording_error_to_control(error: recording_runtime::RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}
