//! Live collector draining, runtime mutation, and SQLite persistence.

use std::time::SystemTime;

#[path = "live/batch.rs"]
mod batch;
#[path = "live/otel_export.rs"]
pub(crate) mod otel_export;
#[path = "live/reconcile.rs"]
mod reconcile;
#[path = "live/tls_debug.rs"]
mod tls_debug;

use collector_event::{RawCollectorEvent, RawObservationPayload};
use collector_instance::CollectorInstance;
use config_core::daemon::DiagnosticLogLevel;
use control_contract::reply::ControlError;
use ingest_runtime::IngestMatch;
use model_core::event::{DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload};
use model_core::ids::{CollectorName, DiagnosticId, EventId, TraceId};
use model_core::process::{ExitStatus, ProcessMembership};
use model_core::trace::TraceLifecycleState;
use store_write_contract::events::EventWriteStore;
use store_write_contract::memberships::MembershipWriteStore;
use store_write_contract::traces::TraceWriteStore;
use trace_runtime::commands::RootRemovalRequest;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::SqliteAttachService;
use crate::services::resource_metrics::COLLECTOR_NAME as RESOURCE_METRICS_COLLECTOR_NAME;

impl SqliteAttachService {
    pub(super) fn drain_live_events_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        self.live_otel_export.check_health()?;
        self.drain_resource_metrics_impl(trace_runtime)?;
        self.drain_tls_sync_events_impl(trace_runtime)?;
        if !self.collector_ready() || self.collector.stats().active_bindings == 0 {
            self.drain_seccomp_notifications_impl(trace_runtime)?;
            self.collector
                .poll_tls_payload_control_events()
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            self.log_tls_diagnostic_events_impl();
            self.ingest_polled_seccomp_tls_controls_impl()?;
            self.drain_seccomp_notifications_impl(trace_runtime)?;
            self.flush_process_seccomp_observations_if_idle(trace_runtime)?;
            self.persist_completed_seccomp_tls_operations_impl(trace_runtime)?;
            self.persist_completed_seccomp_socket_operations_impl(trace_runtime)?;
            self.log_payload_tls_diagnostics_impl()?;
            self.drain_enforcement_impl(trace_runtime)?;
            self.reconcile_draining_memberships_impl(trace_runtime)?;
            return Ok(());
        }

        self.drain_seccomp_notifications_impl(trace_runtime)?;
        let batch = self
            .collector
            .poll_batch()
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        self.log_tls_diagnostic_events_impl();
        self.process_live_event_batch(trace_runtime, batch.observations)?;
        for payload_segment in batch.payload_segments {
            self.process_payload_segment_impl(trace_runtime, payload_segment)?;
        }
        self.ingest_polled_seccomp_tls_controls_impl()?;
        self.drain_seccomp_notifications_impl(trace_runtime)?;
        self.flush_process_seccomp_observations_if_idle(trace_runtime)?;
        self.persist_completed_seccomp_tls_operations_impl(trace_runtime)?;
        self.persist_completed_seccomp_socket_operations_impl(trace_runtime)?;
        self.log_payload_tls_diagnostics_impl()?;
        self.drain_enforcement_impl(trace_runtime)?;
        self.reconcile_draining_memberships_impl(trace_runtime)?;
        Ok(())
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
        for payload_segment in self.tls_sync.drain()? {
            self.process_payload_segment_impl(trace_runtime, payload_segment)?;
        }
        Ok(())
    }

    fn persist_completed_seccomp_tls_operations_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        for payload_segment in self
            .seccomp_tls
            .complete_operations(&self.identity_reader)?
        {
            self.process_payload_segment_impl(trace_runtime, payload_segment)?;
        }
        Ok(())
    }

    fn persist_completed_seccomp_socket_operations_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
    ) -> Result<(), ControlError> {
        let completions = self.collector.take_socket_completions();
        for payload_segment in self.seccomp_socket.complete_operations(completions)? {
            self.process_payload_segment_impl(trace_runtime, payload_segment)?;
        }
        Ok(())
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
        let collector = &self.collector;
        let mut process_observations = Vec::new();
        {
            let process_seccomp = &self.process_seccomp;
            let identity_reader = &self.identity_reader;
            seccomp_notify.drain_notifications(|notification, continuation| {
                process_observations.extend(process_seccomp.handle_notification(
                    identity_reader,
                    notification,
                    continuation,
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

    fn flush_process_seccomp_observations_if_idle(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        if self.seccomp_notify.has_listeners() {
            return Ok(());
        }
        let observations = std::mem::take(&mut self.pending_process_seccomp_observations);
        let raw_events = observations
            .into_iter()
            .map(|observation| self.process_seccomp.materialize_observation(observation))
            .collect();
        self.process_live_event_batch(trace_runtime, raw_events)
    }

    fn drain_resource_metrics_impl(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
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
            self.storage
                .append_event(event.clone())
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            self.persist_semantic_actions_for_event(trace_runtime, &event)?;
        }
        Ok(())
    }

    fn drain_enforcement_impl(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
    ) -> Result<(), ControlError> {
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
                    flags: EventFlags::clean(),
                },
                EventPayload::Enforcement(draft.payload),
            );
            self.storage
                .append_event(event.clone())
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            self.persist_semantic_actions_for_event(trace_runtime, &event)?;
        }
        Ok(())
    }

    pub(super) fn remove_root_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        trace_id: TraceId,
        removed_at: SystemTime,
    ) -> Result<(), ControlError> {
        let root_identity = trace_runtime
            .get_trace(trace_id)
            .map(|entry| entry.trace.root_process_identity.clone())
            .ok_or_else(|| ControlError::new("track_remove", "trace not found"))?;

        trace_runtime
            .track_remove_root(RootRemovalRequest {
                trace_id,
                removed_at,
            })
            .map_err(|error| ControlError::new("track_remove", format!("{:?}", error)))?;
        self.reconcile_draining_memberships_impl(trace_runtime)?;
        self.collector
            .stop_tracking_process(root_identity.pid)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        self.persist_trace_state(trace_runtime, trace_id)?;
        self.log_diagnostic(
            DiagnosticLogLevel::Info,
            format_args!(
                "agent_launch closed trace_id={} pid={} generation={}",
                trace_id, root_identity.pid, root_identity.generation
            ),
        );
        Ok(())
    }

    pub(super) fn next_diagnostic_id(&mut self) -> Result<DiagnosticId, ControlError> {
        let raw = self.next_diagnostic_id;
        self.next_diagnostic_id = self
            .next_diagnostic_id
            .checked_add(1)
            .ok_or_else(|| ControlError::new("diagnostic_id_overflow", "diagnostic id overflow"))?;
        Ok(DiagnosticId::new(raw))
    }

    pub(super) fn next_event_id(&mut self) -> Result<EventId, ControlError> {
        let raw = self.next_event_id;
        self.next_event_id = self
            .next_event_id
            .checked_add(1)
            .ok_or_else(|| ControlError::new("event_id_overflow", "event id overflow"))?;
        Ok(EventId::new(raw))
    }

    fn apply_runtime_effects(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        raw_event: &RawCollectorEvent,
    ) -> Result<Option<IngestMatch>, ControlError> {
        match &raw_event.payload {
            RawObservationPayload::Process {
                operation, parent, ..
            } if operation == "fork" => {
                let Some(parent_identity) = parent else {
                    return Ok(None);
                };
                let Some((trace_id, _)) = trace_runtime.find_membership(&parent_identity) else {
                    return Ok(None);
                };
                trace_runtime
                    .insert_observed_child(
                        trace_id,
                        &parent_identity,
                        raw_event.envelope.process.clone(),
                    )
                    .map_err(|error| {
                        ControlError::new("insert_observed_child", format!("{:?}", error))
                    })?;
                Ok(Some(IngestMatch {
                    trace_id,
                    process: raw_event.envelope.process.clone(),
                }))
            }
            RawObservationPayload::Process {
                operation,
                parent,
                metadata,
            } if operation == "exec" => {
                let identity = raw_event.envelope.process.clone();
                if let Some((trace_id, membership)) = trace_runtime.find_membership(&identity) {
                    return Ok(Some(IngestMatch {
                        trace_id,
                        process: membership.identity,
                    }));
                }
                if let Some(parent_identity) = parent {
                    if let Some((trace_id, parent_membership)) =
                        trace_runtime.find_membership_by_pid(parent_identity.pid)
                    {
                        trace_runtime
                            .insert_observed_child(
                                trace_id,
                                &parent_membership.identity,
                                identity.clone(),
                            )
                            .map_err(|error| {
                                ControlError::new("insert_observed_child", format!("{:?}", error))
                            })?;
                        return Ok(Some(IngestMatch {
                            trace_id,
                            process: identity,
                        }));
                    }
                }
                if let Some(parent_pid) = metadata.get("ppid").and_then(|value| value.parse().ok())
                {
                    if let Some((trace_id, parent_membership)) =
                        trace_runtime.find_membership_by_pid(parent_pid)
                    {
                        trace_runtime
                            .insert_observed_child(
                                trace_id,
                                &parent_membership.identity,
                                identity.clone(),
                            )
                            .map_err(|error| {
                                ControlError::new("insert_observed_child", format!("{:?}", error))
                            })?;
                        return Ok(Some(IngestMatch {
                            trace_id,
                            process: identity,
                        }));
                    }
                }
                Ok(trace_runtime
                    .refresh_process_identity(identity.clone())
                    .map(|(trace_id, process)| IngestMatch { trace_id, process }))
            }
            RawObservationPayload::Process {
                operation,
                metadata,
                ..
            } if operation == "exit" => {
                let Some((trace_id, membership)) =
                    trace_runtime.find_membership(&raw_event.envelope.process)
                else {
                    return Ok(None);
                };
                trace_runtime
                    .mark_process_exited(
                        trace_id,
                        &membership.identity,
                        ExitStatus {
                            code: exit_code(&metadata)?,
                            observed_at: raw_event.envelope.observed_at,
                        },
                    )
                    .map_err(|error| {
                        ControlError::new("mark_process_exited", format!("{:?}", error))
                    })?;
                Ok(Some(IngestMatch {
                    trace_id,
                    process: membership.identity,
                }))
            }
            _ => Ok(trace_runtime
                .find_membership(&raw_event.envelope.process)
                .map(|(trace_id, membership)| IngestMatch {
                    trace_id,
                    process: membership.identity,
                })),
        }
    }

    pub(super) fn persist_trace_state(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
    ) -> Result<(), ControlError> {
        let (trace, memberships, terminal) = trace_runtime
            .get_trace(trace_id)
            .map(|entry| {
                (
                    entry.trace.clone(),
                    entry
                        .memberships
                        .memberships()
                        .cloned()
                        .collect::<Vec<ProcessMembership>>(),
                    matches!(
                        entry.trace.lifecycle_state,
                        TraceLifecycleState::Completed | TraceLifecycleState::Failed
                    ),
                )
            })
            .ok_or_else(|| ControlError::new("persist_trace_state", "trace not found"))?;

        self.storage
            .create_trace(trace)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        for membership in memberships {
            self.storage
                .upsert_membership(membership)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }

        if terminal {
            self.collector
                .unbind_trace(trace_id)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }

        Ok(())
    }
}

fn exit_code(
    metadata: &std::collections::BTreeMap<String, String>,
) -> Result<Option<i32>, ControlError> {
    metadata
        .get("exit_code")
        .map(|value| {
            value
                .parse::<i32>()
                .map_err(|error| ControlError::new("exit_code", error.to_string()))
        })
        .transpose()
}
