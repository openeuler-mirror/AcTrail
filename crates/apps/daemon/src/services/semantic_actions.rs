//! Daemon wiring for live semantic action materialization.

use std::time::SystemTime;

use control_contract::reply::ControlError;
use model_core::diagnostics::DiagnosticRecord;
use model_core::diagnostics::LlmPipelineDiagnostic;
use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::process::{ProcessIdentity, ProcessRecord};
use model_core::trace::TraceRecord;
use recording_runtime::{
    RecordingError, RecordingWriter, SemanticActionBatch, TraceRecordLookup, TraceStateRecord,
};
use semantic_action::SemanticActionKind;
use semantic_action::SemanticActionLink;
use semantic_action_runtime::derive_lineage_links;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;
use crate::services::live::next_diagnostic_id_from_seed;

impl StorageAttachService {
    pub(super) fn recognized_agent_processes(
        batch: &SemanticActionBatch,
    ) -> Vec<(TraceId, ProcessIdentity)> {
        batch
            .actions()
            .iter()
            .filter(|action| action.kind == SemanticActionKind::AgentIdentity)
            .map(|action| (action.trace_id, action.process))
            .collect()
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

    pub(super) fn persisted_agent_processes_after_failure(
        &self,
        agents: Vec<(TraceId, ProcessIdentity)>,
    ) -> Vec<(TraceId, ProcessIdentity)> {
        agents
            .into_iter()
            .filter(
                |(trace_id, process)| match self.storage.list_semantic_actions(*trace_id) {
                    Ok(actions) => actions.iter().any(|action| {
                        action.kind == SemanticActionKind::AgentIdentity
                            && action.process == *process
                    }),
                    Err(error) => {
                        tracing::warn!(
                            trace_id = trace_id.get(),
                            process_id = process.get(),
                            stage = %error.stage,
                            message = %error.message,
                            "Agent observation depth persistence check failed locally"
                        );
                        false
                    }
                },
            )
            .collect()
    }

    pub(super) fn persist_observed_batch_then_publish(
        &mut self,
        trace_runtime: &TraceRuntime,
        events: Vec<DomainEvent>,
        mut diagnostics: Vec<DiagnosticRecord>,
        mut semantic_actions: SemanticActionBatch,
        trace_states: Vec<TraceStateRecord>,
        process_records: Vec<ProcessRecord>,
    ) -> Result<(), ControlError> {
        let observed_at = SystemTime::now();
        let idle_update = self
            .idle_runtime
            .prepare_batch(&mut semantic_actions, observed_at);
        diagnostics.extend(
            self.idle_attribution_diagnostics(idle_update.ambiguous_traces(), observed_at)?,
        );
        let event_count = events.len();
        let diagnostic_count = diagnostics.len();
        let semantic_action_count = semantic_actions.actions().len();
        let semantic_link_count = semantic_actions.links().len();
        let trace_state_count = trace_states.len();
        let traces = LiveTraceRecordLookup::new(trace_runtime);
        let started = crate::services::workload_diagnostics::now();
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        let (result, persisted) = RecordingWriter::new(self.storage.as_mut())
            .persist_live_events_then_export_with_additional_write(
                &self.export_runtime,
                events,
                diagnostics,
                semantic_actions,
                trace_states,
                process_records,
                |storage| idle_update.persist(storage),
                &traces,
                observed_at,
                || {
                    next_diagnostic_id_from_seed(next_diagnostic_id)
                        .map_err(control_error_to_recording)
                },
            );
        if persisted {
            self.idle_runtime.commit(idle_update);
        }
        let result = result.map_err(recording_error_to_control);
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
    ) -> (SemanticActionBatch, Vec<LlmPipelineDiagnostic>) {
        let mut output = self.semantic_actions.finalize_trace(trace_id, finished_at);
        let diagnostics = std::mem::take(&mut output.llm_pipeline_diagnostics);
        (
            SemanticActionBatch::from_action_output(
                output.actions,
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
        // Attribute final trace actions before projecting and persisting them.
        let (mut semantic_actions, llm_pipeline_diagnostics) =
            self.finalize_semantic_actions_for_trace(trace_id, finished_at);
        let idle_update =
            self.idle_runtime
                .prepare_terminal_batch(&mut semantic_actions, finished_at, trace_id);
        let mut export_batch = semantic_actions.clone();
        let mut errors = Vec::new();
        let mut final_batch_persisted = false;
        match RecordingWriter::new(self.storage.as_mut())
            .persist_semantic_actions_with_additional_write(semantic_actions, |storage| {
                idle_update.persist(storage)
            })
            .map_err(recording_error_to_control)
        {
            Ok(()) => {
                final_batch_persisted = true;
                self.idle_runtime.commit(idle_update);
                match self.rebuild_lineage_semantic_links(trace_id) {
                    Ok(lineage_links) => {
                        export_batch
                            .extend(SemanticActionBatch::from_parts(Vec::new(), lineage_links));
                    }
                    Err(error) => errors.push(error),
                }
            }
            Err(error) => errors.push(error),
        }

        if final_batch_persisted {
            if let Err(error) =
                self.publish_live_export_actions(trace_runtime, trace_id, export_batch)
            {
                errors.push(error);
            }
        }

        self.persist_llm_pipeline_diagnostics_fail_local(trace_runtime, llm_pipeline_diagnostics);

        combine_control_errors(errors)
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
        trace_id: TraceId,
        semantic_actions: SemanticActionBatch,
    ) -> Result<(), ControlError> {
        let traces = LiveTraceRecordLookup::new(trace_runtime);
        let next_diagnostic_id = &mut self.next_diagnostic_id;
        RecordingWriter::new(self.storage.as_mut())
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
            )
            .map_err(recording_error_to_control)
    }
}

fn combine_control_errors(errors: Vec<ControlError>) -> Result<(), ControlError> {
    if errors.is_empty() {
        return Ok(());
    }
    let message = errors
        .iter()
        .map(|error| format!("{}: {}", error.code, error.message))
        .collect::<Vec<_>>()
        .join("; ");
    Err(ControlError::new("semantic_action_finalize", message))
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
