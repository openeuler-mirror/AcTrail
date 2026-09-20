//! Daemon wiring for live semantic action materialization.

use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::diagnostics::DiagnosticRecord;
use model_core::diagnostics::LlmPipelineDiagnostic;
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessMembership, ProcessRecord};
use model_core::trace::TraceRecord;
use recording_runtime::{
    DeliveryFailureKind, DeliveryReport, RecordingError, RecordingWriter, SemanticActionBatch,
    TraceRecordLookup, TraceStateRecord,
};
use semantic_action::SemanticActionKind;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::live::next_diagnostic_id_from_seed;

impl StorageAttachService {
    pub(super) fn recognized_agent_processes(
        batch: &SemanticActionBatch,
    ) -> Vec<(TraceId, ProcessIdentity)> {
        let mut agents = Vec::new();
        Self::append_recognized_agent_processes(batch, &mut agents);
        agents
    }

    pub(super) fn append_recognized_agent_processes(
        batch: &SemanticActionBatch,
        agents: &mut Vec<(TraceId, ProcessIdentity)>,
    ) {
        agents.extend(
            batch
                .actions()
                .iter()
                .filter(|action| action.kind == SemanticActionKind::AgentIdentity)
                .map(|action| (action.trace_id, action.process)),
        );
    }

    pub(super) fn apply_agent_observation_depths(
        &mut self,
        trace_runtime: &TraceRuntime,
        agents: Vec<(TraceId, ProcessIdentity)>,
    ) {
        for (trace_id, process) in agents {
            let Some(depth) = trace_runtime
                .get_trace(trace_id)
                .map(|entry| entry.profile_snapshot.agent_descendant_observation_depth)
            else {
                tracing::warn!(
                    trace_id = trace_id.get(),
                    "Agent observation depth update skipped because trace state is missing"
                );
                continue;
            };
            if depth == -1 {
                continue;
            }
            let Some(host) = self
                .process_registry
                .record(process)
                .and_then(|record| record.host.as_ref())
            else {
                tracing::warn!(
                    trace_id = trace_id.get(),
                    process_id = process.get(),
                    "Agent observation depth update skipped because observer TGID is missing"
                );
                continue;
            };
            let observer_tgid = host.pid;
            let expected_start_boottime_ns = host.start_boottime_ns;
            let expected_start_time_ticks = host.start_time_ticks;
            if let Err(error) = self.collector.set_agent_descendant_observation_depth(
                trace_id,
                observer_tgid,
                expected_start_boottime_ns,
                expected_start_time_ticks,
                depth,
            ) {
                tracing::warn!(
                    trace_id = trace_id.get(),
                    process_id = process.get(),
                    observer_tgid,
                    stage = %error.stage,
                    message = %error.message,
                    "Agent observation depth update failed locally"
                );
            }
        }
    }

    pub(super) fn write_semantic_action_batch(&mut self, batch: SemanticActionBatch) {
        if let Err(error) =
            RecordingWriter::new(self.storage.as_mut()).persist_semantic_actions(batch)
        {
            tracing::warn!(stage = %error.stage, message = %error.message,
                "Semantic action storage failed locally");
        }
    }

    pub(super) fn persist_observed_batch_then_publish(
        &mut self,
        trace_runtime: &TraceRuntime,
        events: Vec<DomainEvent>,
        diagnostics: Vec<DiagnosticRecord>,
        semantic_actions: SemanticActionBatch,
        trace_states: Vec<TraceStateRecord>,
        memberships: Vec<ProcessMembership>,
        process_records: Vec<ProcessRecord>,
    ) -> Result<(), ControlError> {
        let event_count = events.len();
        let diagnostic_count = diagnostics.len();
        let semantic_action_count = semantic_actions.action_views().count();
        let semantic_link_count = semantic_actions.links().len();
        let trace_state_count = trace_states.len();
        self.apply_agent_observation_depths(
            trace_runtime,
            Self::recognized_agent_processes(&semantic_actions),
        );
        let traces = LiveTraceRecordLookup::new(trace_runtime);
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        let started = crate::services::workload_diagnostics::now();
        let result = RecordingWriter::new(self.storage.as_mut()).persist_live_events_then_export(
            &self.export_runtime,
            events,
            diagnostics,
            semantic_actions,
            trace_states,
            memberships,
            process_records,
            &traces,
            SystemTime::now(),
            || next_diagnostic_id_from_seed(next_diagnostic_id).map_err(control_error_to_recording),
        );
        self.workload_diagnostics.record_storage_batch(
            started.elapsed(),
            event_count,
            0,
            diagnostic_count,
            semantic_action_count,
            semantic_link_count,
            trace_state_count,
            result.storage_succeeded(),
        );
        Self::handle_delivery_report(result)
    }

    pub(super) fn mark_semantic_projection_dirty(&mut self, trace_id: TraceId) {
        self.finalized_terminal_traces.remove(&trace_id);
    }

    pub(super) fn finalize_semantic_actions_for_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: std::time::SystemTime,
    ) -> (SemanticActionBatch, Vec<LlmPipelineDiagnostic>) {
        let mut output = self.semantic_actions.finalize_trace(trace_id, finished_at);
        let diagnostics = std::mem::take(&mut output.llm_pipeline_diagnostics);
        (
            SemanticActionBatch::from_action_output(
                output.actions,
                output.updates,
                output.updated_actions,
                output.links,
                output.file_observation_paths,
                output.file_path_sets,
                output.llm_request_contents,
                output.llm_request_lineages,
                output.mcp_jsonrpc_contents,
                output.payload_segments,
            ),
            diagnostics,
        )
    }

    pub(super) fn finalize_semantic_projection_for_trace(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
        finished_at: std::time::SystemTime,
    ) -> Result<(), ControlError> {
        let (semantic_actions, llm_pipeline_diagnostics) =
            self.finalize_semantic_actions_for_trace(trace_id, finished_at);
        let export_batch = self
            .export_runtime
            .has_semantic_consumers()
            .then(|| semantic_actions.clone());
        self.write_semantic_action_batch(semantic_actions);
        let result = match export_batch {
            Some(batch) => self.publish_live_export_actions(trace_runtime, trace_id, batch),
            None => Ok(()),
        };

        self.persist_llm_pipeline_diagnostics_fail_local(trace_runtime, llm_pipeline_diagnostics);

        result
    }

    pub(super) fn publish_live_export_actions(
        &mut self,
        trace_runtime: &TraceRuntime,
        trace_id: TraceId,
        semantic_actions: SemanticActionBatch,
    ) -> Result<(), ControlError> {
        let traces = LiveTraceRecordLookup::new(trace_runtime);
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        let report = RecordingWriter::new(self.storage.as_mut())
            .export_final_semantic_action_batch_for_trace(
                &self.export_runtime,
                &traces,
                trace_id,
                semantic_actions,
                SystemTime::now(),
                || {
                    next_diagnostic_id_from_seed(next_diagnostic_id)
                        .map_err(control_error_to_recording)
                },
            );
        Self::handle_delivery_report(report)
    }

    pub(in crate::services) fn handle_delivery_report(
        report: DeliveryReport,
    ) -> Result<(), ControlError> {
        let mut runtime_error = None;
        for failure in report.into_failures() {
            tracing::warn!(
                kind = ?failure.kind,
                stage = %failure.error.stage,
                message = %failure.error.message,
                "Observation delivery failed locally"
            );
            if failure.kind == DeliveryFailureKind::Runtime && runtime_error.is_none() {
                runtime_error = Some(recording_error_to_control(failure.error));
            }
        }
        runtime_error.map_or(Ok(()), Err)
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
