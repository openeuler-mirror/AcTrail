//! Daemon wiring for live semantic action materialization.

use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::trace::TraceRecord;
use recording_runtime::{
    RecordingError, RecordingWriter, SemanticActionBatch, TraceRecordLookup, TraceStateRecord,
};
use semantic_action::{SemanticAction, SemanticActionLink};
use semantic_action_runtime::derive_lineage_links;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::live::next_diagnostic_id_from_seed;

impl StorageAttachService {
    pub(super) fn write_semantic_action_batch(
        &mut self,
        batch: SemanticActionBatch,
    ) -> Result<SemanticActionBatch, ControlError> {
        RecordingWriter::new(self.storage.as_mut())
            .persist_semantic_actions(batch)
            .map_err(recording_error_to_control)
    }

    pub(super) fn persist_observed_batch_then_publish(
        &mut self,
        trace_runtime: &TraceRuntime,
        events: Vec<DomainEvent>,
        diagnostics: Vec<DiagnosticRecord>,
        semantic_actions: SemanticActionBatch,
        trace_states: Vec<TraceStateRecord>,
    ) -> Result<(), ControlError> {
        let event_count = events.len();
        let diagnostic_count = diagnostics.len();
        let semantic_action_count = semantic_actions.actions().len();
        let semantic_link_count = semantic_actions.links().len();
        let trace_state_count = trace_states.len();
        let traces = LiveTraceRecordLookup::new(trace_runtime);
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        let started = crate::services::workload_diagnostics::now();
        let result = RecordingWriter::new(self.storage.as_mut())
            .persist_live_events_then_export(
                &self.export_runtime,
                events,
                diagnostics,
                semantic_actions,
                trace_states,
                &traces,
                SystemTime::now(),
                || {
                    next_diagnostic_id_from_seed(next_diagnostic_id)
                        .map_err(control_error_to_recording)
                },
            )
            .map_err(recording_error_to_control);
        self.workload_diagnostics.record_storage_batch(
            started.elapsed(),
            event_count,
            0,
            diagnostic_count,
            semantic_action_count,
            semantic_link_count,
            trace_state_count,
            result.is_ok(),
        );
        result
    }

    pub(super) fn mark_semantic_projection_dirty(&mut self, trace_id: TraceId) {
        self.finalized_terminal_traces.remove(&trace_id);
    }

    pub(super) fn finalize_semantic_actions_for_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: std::time::SystemTime,
    ) -> SemanticActionBatch {
        let output = self.semantic_actions.finalize_trace(trace_id, finished_at);
        SemanticActionBatch::from_action_output(
            output.actions,
            output.links,
            output.file_observation_paths,
            output.file_path_sets,
        )
    }

    pub(super) fn finalize_semantic_projection_for_trace(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
        finished_at: std::time::SystemTime,
    ) -> Result<(), ControlError> {
        let semantic_actions = self.finalize_semantic_actions_for_trace(trace_id, finished_at);
        let semantic_actions = self.write_semantic_action_batch(semantic_actions)?;
        let mut links = semantic_actions.links().to_vec();
        links.extend(self.rebuild_lineage_semantic_links(trace_id)?);
        self.publish_live_export_actions(trace_runtime, semantic_actions.actions(), &links)
    }

    pub(super) fn rebuild_lineage_semantic_links(
        &mut self,
        trace_id: TraceId,
    ) -> Result<Vec<SemanticActionLink>, ControlError> {
        let memberships = self
            .storage
            .trace_memberships(trace_id)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let actions = self
            .storage
            .list_semantic_actions(trace_id)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let existing_links = self
            .storage
            .list_semantic_action_links(trace_id)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let links = derive_lineage_links(trace_id, &memberships, &actions, &existing_links);
        let batch = SemanticActionBatch::from_parts(Vec::new(), links);
        let batch = RecordingWriter::new(self.storage.as_mut())
            .persist_semantic_actions(batch)
            .map_err(recording_error_to_control)?;
        let (_, links) = batch.into_parts();
        Ok(links)
    }

    pub(super) fn publish_live_export_actions(
        &mut self,
        trace_runtime: &TraceRuntime,
        actions: &[SemanticAction],
        links: &[SemanticActionLink],
    ) -> Result<(), ControlError> {
        let traces = LiveTraceRecordLookup::new(trace_runtime);
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        RecordingWriter::new(self.storage.as_mut())
            .export_semantic_actions_for_trace(
                &self.export_runtime,
                &traces,
                actions,
                links,
                SystemTime::now(),
                || {
                    next_diagnostic_id_from_seed(next_diagnostic_id)
                        .map_err(control_error_to_recording)
                },
            )
            .map_err(recording_error_to_control)
    }
}

fn recording_error_to_control(error: RecordingError) -> ControlError {
    ControlError::new(error.stage, error.message)
}

fn control_error_to_recording(error: ControlError) -> RecordingError {
    RecordingError::new(error.code, error.message)
}

pub(in crate::services) struct LiveTraceRecordLookup<'a> {
    trace_runtime: &'a TraceRuntime,
}

impl<'a> LiveTraceRecordLookup<'a> {
    pub(in crate::services) fn new(trace_runtime: &'a TraceRuntime) -> Self {
        Self { trace_runtime }
    }
}

impl TraceRecordLookup for LiveTraceRecordLookup<'_> {
    fn trace_record(&self, trace_id: TraceId) -> Option<&TraceRecord> {
        // Keep TraceRuntime ownership in daemon while recording sees only trace records.
        self.trace_runtime
            .get_trace(trace_id)
            .map(|entry| &entry.trace)
    }
}
