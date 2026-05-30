//! Daemon wiring for live semantic action materialization.

use std::collections::BTreeMap;
use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use semantic_action::SemanticAction;
use semantic_action::SemanticActionWriteStore;
use store_write_contract::diagnostics::DiagnosticWriteStore;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::SqliteAttachService;

impl SqliteAttachService {
    pub(super) fn persist_semantic_actions_for_event(
        &mut self,
        trace_runtime: &TraceRuntime,
        event: &DomainEvent,
    ) -> Result<(), ControlError> {
        let actions = self.semantic_actions.observe_event(event);
        for action in actions.iter().cloned() {
            self.storage
                .upsert_semantic_action(action)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        self.publish_live_otel_actions(trace_runtime, &actions)?;
        Ok(())
    }

    pub(super) fn publish_live_otel_actions(
        &mut self,
        trace_runtime: &TraceRuntime,
        actions: &[SemanticAction],
    ) -> Result<(), ControlError> {
        self.live_otel_export.check_health()?;
        if !self.live_otel_export.enabled() || actions.is_empty() {
            return Ok(());
        }
        let mut dropped_by_trace = BTreeMap::<TraceId, u64>::new();
        for action in actions {
            let trace = trace_runtime
                .get_trace(action.trace_id)
                .ok_or_else(|| ControlError::new("otel_live_export", "trace not found"))?;
            let result = self.live_otel_export.publish(&trace.trace, action)?;
            if result.dropped_spans() > u64::default() {
                dropped_by_trace
                    .entry(action.trace_id)
                    .and_modify(|count| *count = count.saturating_add(result.dropped_spans()))
                    .or_insert_with(|| result.dropped_spans());
            }
        }
        for (trace_id, dropped_spans) in dropped_by_trace {
            self.append_live_otel_drop_diagnostic(trace_id, dropped_spans)?;
        }
        Ok(())
    }

    fn append_live_otel_drop_diagnostic(
        &mut self,
        trace_id: TraceId,
        dropped_spans: u64,
    ) -> Result<(), ControlError> {
        let diagnostic = DiagnosticRecord::new(
            self.next_diagnostic_id()?,
            Some(trace_id),
            DiagnosticKind::RuntimeDropped,
            DiagnosticSeverity::Warning,
            SystemTime::now(),
            "live OTEL export dropped spans because the configured queue was full",
        )
        .with_metadata("sink", "otel_live_jsonl")
        .with_metadata("dropped_spans", dropped_spans.to_string())
        .with_metadata(
            "queue_capacity",
            self.live_otel_export.queue_capacity().to_string(),
        );
        self.storage
            .append_diagnostic(diagnostic)
            .map_err(|error| ControlError::new(error.stage, error.message))
    }
}
