//! Reconcile draining traces against procfs when lifecycle exit events were missed.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::event::{
    DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, ProcessPayload,
};
use model_core::ids::CollectorName;
use model_core::process::{ExitObservationSource, ExitStatus, MembershipState, ProcessIdentity};
use model_core::trace::TraceLifecycleState;
use process_identity::{IdentityLookupError, ProcessIdentityReader};
use recording_runtime::RecordingWriter;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;

const RECONCILE_COLLECTOR_NAME: &str = "process-reconcile";
const EXIT_SOURCE_RECONCILED: &str = "reconciled";
const PROCESS_OPERATION_EXIT: &str = "exit";

impl StorageAttachService {
    pub(in crate::services) fn reconcile_draining_memberships_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let trace_ids = trace_runtime
            .list_trace_records()
            .into_iter()
            .filter(|trace| {
                trace.lifecycle_state == TraceLifecycleState::Draining
                    || trace.lifecycle_state.is_terminal()
            })
            .map(|trace| trace.trace_id)
            .collect::<Vec<_>>();
        let mut touched_traces = BTreeSet::new();

        for trace_id in trace_ids {
            let candidates = trace_runtime
                .get_trace(trace_id)
                .map(|entry| {
                    entry
                        .memberships
                        .memberships()
                        .filter(|membership| {
                            membership.capture_enabled
                                && matches!(
                                    membership.state,
                                    MembershipState::Starting | MembershipState::Active
                                )
                        })
                        .map(|membership| membership.identity.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for identity in candidates {
                if !self.process_membership_is_gone(&identity) {
                    if terminal_trace(trace_runtime, trace_id) {
                        self.record_terminal_open_membership(trace_runtime, trace_id, &identity)?;
                        touched_traces.insert(trace_id);
                    }
                    continue;
                }
                let observed_at = SystemTime::now();
                trace_runtime
                    .mark_process_exited(
                        trace_id,
                        &identity,
                        ExitStatus {
                            code: None,
                            observed_at,
                            source: Some(ExitObservationSource::Reconciled),
                        },
                    )
                    .map_err(|error| {
                        ControlError::new("reconcile_draining_membership", format!("{:?}", error))
                    })?;
                let event = self.reconciled_exit_event(trace_id, identity, observed_at)?;
                self.persist_observed_event_batch(trace_runtime, vec![event])?;
                touched_traces.insert(trace_id);
            }
        }

        for trace_id in touched_traces {
            self.persist_trace_state(trace_runtime, trace_id)?;
        }
        Ok(())
    }

    fn record_terminal_open_membership(
        &mut self,
        trace_runtime: &mut TraceRuntime,
        trace_id: model_core::ids::TraceId,
        identity: &ProcessIdentity,
    ) -> Result<(), ControlError> {
        if !self
            .diagnosed_terminal_open_memberships
            .insert((trace_id, identity.clone()))
        {
            return Ok(());
        }
        trace_runtime.mark_degraded(trace_id).map_err(|error| {
            ControlError::new("terminal_membership_degraded", format!("{error:?}"))
        })?;
        let diagnostic = DiagnosticRecord::new(
            self.next_diagnostic_id()?,
            Some(trace_id),
            DiagnosticKind::RuntimeFatal,
            DiagnosticSeverity::Error,
            SystemTime::now(),
            "terminal trace still has a live process membership",
        )
        .with_process(*identity)
        .with_metadata("process_id", identity.get().to_string());
        let diagnostic = match self
            .process_registry
            .record(*identity)
            .and_then(|record| record.host.as_ref())
        {
            Some(host) => diagnostic
                .with_metadata("host_pid", host.pid.to_string())
                .with_metadata("start_time_ticks", host.start_time_ticks.to_string()),
            None => diagnostic,
        };
        RecordingWriter::new(self.storage.as_mut())
            .persist_diagnostic(diagnostic)
            .map_err(recording_error_to_control)
    }

    fn reconciled_exit_event(
        &mut self,
        trace_id: model_core::ids::TraceId,
        process: ProcessIdentity,
        observed_at: SystemTime,
    ) -> Result<DomainEvent, ControlError> {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "exit_source".to_string(),
            EXIT_SOURCE_RECONCILED.to_string(),
        );
        Ok(DomainEvent::new(
            EventEnvelope {
                event_id: self.next_event_id()?,
                trace_id,
                observed_at,
                process,
                collector: CollectorName::new(RECONCILE_COLLECTOR_NAME),
                kind: EventKind::Process,
                flags: EventFlags::clean(),
            },
            EventPayload::Process(ProcessPayload {
                operation: PROCESS_OPERATION_EXIT.to_string(),
                parent: None,
                executable: None,
                metadata,
            }),
        ))
    }

    fn process_membership_is_gone(&self, identity: &ProcessIdentity) -> bool {
        let Some(record) = self.process_registry.record(*identity) else {
            return false;
        };
        let Some(host) = record.host.as_ref() else {
            return false;
        };
        match self.identity_reader.read_identity(host.pid) {
            Ok(current) => current
                .host
                .as_ref()
                .is_none_or(|current_host| current_host.start_time_ticks != host.start_time_ticks),
            Err(IdentityLookupError::NotFound { .. }) => true,
            Err(IdentityLookupError::PermissionDenied { .. })
            | Err(IdentityLookupError::Incomplete { .. }) => false,
        }
    }
}

fn recording_error_to_control(error: recording_runtime::RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}

fn terminal_trace(trace_runtime: &TraceRuntime, trace_id: model_core::ids::TraceId) -> bool {
    trace_runtime
        .get_trace(trace_id)
        .is_some_and(|entry| entry.trace.lifecycle_state.is_terminal())
}
