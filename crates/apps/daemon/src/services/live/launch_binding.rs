//! Trace-scoped persistence for kernel launch-binding failures.

use std::collections::BTreeSet;
use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use recording_runtime::{SemanticActionBatch, TraceStateRecord};
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;

impl StorageAttachService {
    pub(super) fn persist_launch_binding_failures_impl(
        &mut self,
        trace_runtime: &mut TraceRuntime,
    ) -> Result<(), ControlError> {
        let failures = self.collector.take_launch_binding_failures();
        if failures.is_empty() {
            return Ok(());
        }

        let emitted_at = SystemTime::now();
        let mut affected_traces = BTreeSet::new();
        let mut diagnostics = Vec::with_capacity(failures.len());
        for failure in failures {
            let trace_id = failure.trace_id();
            if trace_runtime.get_trace(trace_id).is_none() {
                tracing::warn!(
                    %trace_id,
                    status = failure.status().code(),
                    "launch binding failure references an unknown trace"
                );
                continue;
            }
            trace_runtime.mark_degraded(trace_id).map_err(|error| {
                ControlError::new("launch_binding_degrade", format!("{error:?}"))
            })?;
            affected_traces.insert(trace_id);
            diagnostics.push(DiagnosticRecord::new(
                self.next_diagnostic_id()?,
                Some(trace_id),
                DiagnosticKind::RuntimeFailure,
                DiagnosticSeverity::Error,
                emitted_at,
                failure.status().code(),
            ));
        }
        let trace_states = affected_traces
            .into_iter()
            .map(|trace_id| self.trace_state_record_for_persistence(trace_runtime, trace_id))
            .collect::<Result<Vec<TraceStateRecord>, _>>()?;
        self.persist_observed_batch_then_publish(
            trace_runtime,
            Vec::new(),
            diagnostics,
            SemanticActionBatch::default(),
            trace_states,
            Vec::new(),
        )
    }
}
