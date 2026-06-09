//! Daemon wiring for live semantic action materialization.

use std::collections::BTreeMap;
use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::payload::PayloadSegment;
use semantic_action::SemanticActionWriteStore;
use semantic_action::{SemanticAction, SemanticActionLink};
use store_write_contract::diagnostics::DiagnosticWriteStore;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::SqliteAttachService;

impl SqliteAttachService {
    pub(super) fn observe_semantic_actions_for_event(
        &mut self,
        event: &DomainEvent,
    ) -> SemanticActionBatch {
        let output = self.semantic_actions.observe_event(event);
        SemanticActionBatch {
            actions: output.actions,
            links: output.links,
        }
    }

    pub(super) fn observe_semantic_actions_for_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> SemanticActionBatch {
        let output = self.semantic_actions.observe_payload_segment(segment);
        SemanticActionBatch {
            actions: output.actions,
            links: output.links,
        }
    }

    pub(super) fn write_semantic_action_batch(
        &mut self,
        batch: &SemanticActionBatch,
    ) -> Result<(), ControlError> {
        for action in batch.actions.iter().cloned() {
            self.storage
                .upsert_semantic_action(action)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        for link in batch.links.iter().cloned() {
            self.storage
                .upsert_semantic_action_link(link)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
        }
        Ok(())
    }

    pub(super) fn finalize_semantic_actions_for_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: std::time::SystemTime,
    ) -> SemanticActionBatch {
        let output = self.semantic_actions.finalize_trace(trace_id, finished_at);
        SemanticActionBatch {
            actions: output.actions,
            links: output.links,
        }
    }

    pub(super) fn publish_live_otel_action_batch(
        &mut self,
        trace_runtime: &TraceRuntime,
        batch: &SemanticActionBatch,
    ) -> Result<(), ControlError> {
        self.publish_live_otel_actions(trace_runtime, &batch.actions, &batch.links)
    }

    pub(super) fn publish_live_otel_actions(
        &mut self,
        trace_runtime: &TraceRuntime,
        actions: &[SemanticAction],
        links: &[SemanticActionLink],
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
            let result = self.live_otel_export.publish(&trace.trace, action, links)?;
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

#[derive(Default)]
pub(super) struct SemanticActionBatch {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) links: Vec<SemanticActionLink>,
}

impl SemanticActionBatch {
    pub(super) fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.links.extend(other.links);
    }
}
