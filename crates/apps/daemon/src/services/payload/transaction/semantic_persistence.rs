use super::*;
use semantic_action::SemanticEvidenceKind;

impl PayloadTransactionContext<'_> {
    pub(super) fn observe_payload_gap(&mut self, segment: &PayloadSegment) -> SemanticActionBatch {
        let observation = self.semantic_actions.observe_payload_gap(segment);
        self.mcp_stdio_diagnostics
            .extend(observation.mcp_stdio_diagnostics);
        let output = observation.output;
        SemanticActionBatch::from_action_output(
            output.actions,
            output.links,
            output.file_observation_paths,
            output.file_path_sets,
            output.llm_request_contents,
            output.llm_request_lineages,
            output.mcp_jsonrpc_contents,
            output.payload_segments,
        )
    }

    pub(super) fn observe_semantic_actions_for_event(
        &mut self,
        event: &DomainEvent,
    ) -> SemanticActionBatch {
        let observation = self.semantic_actions.observe_event_with_diagnostics(event);
        self.mcp_stdio_diagnostics
            .extend(observation.mcp_stdio_diagnostics);
        let output = observation.output;
        SemanticActionBatch::from_action_output(
            output.actions,
            output.links,
            output.file_observation_paths,
            output.file_path_sets,
            output.llm_request_contents,
            output.llm_request_lineages,
            output.mcp_jsonrpc_contents,
            output.payload_segments,
        )
    }

    pub(super) fn persist_prepared_payload_segments(
        &mut self,
        session: &mut ObservedRecordWriteSession<'_>,
        prepared: Vec<PreparedPayloadSegment>,
    ) -> Result<(), ControlError> {
        for prepared in prepared {
            match prepared {
                PreparedPayloadSegment::Retained {
                    stored_segment,
                    semantic_actions,
                    application_events,
                    next_retained_bytes,
                    retained_body_bytes,
                } => {
                    let semantic_action_count = semantic_actions.actions().len();
                    let trace_id = stored_segment.trace_id;
                    let process_id = stored_segment.process.get();
                    let source_boundary = stored_segment.source_boundary;
                    let captured_size = stored_segment.captured_size;
                    let operation_id = stored_segment.operation_id;
                    let started = crate::services::workload_diagnostics::now();
                    session
                        .persist_payload_segment(stored_segment, semantic_actions)
                        .map_err(recording_error_to_control)?;
                    self.workload_diagnostics.record_payload_transaction_phase(
                        PayloadTransactionPhase::SegmentPersist,
                        started.elapsed(),
                        semantic_action_count,
                    );
                    self.log_payload_diagnostic(format_args!(
                        "payload_persist staged trace_id={} process_id={} source={:?} captured_bytes={} retained_body_bytes={} operation_id={}",
                        trace_id,
                        process_id,
                        source_boundary,
                        captured_size,
                        retained_body_bytes,
                        operation_id
                    ));
                    let application_event_count = application_events.len();
                    let started = crate::services::workload_diagnostics::now();
                    for prepared_event in application_events {
                        session
                            .persist_event(prepared_event.event, prepared_event.semantic_actions)
                            .map_err(recording_error_to_control)?;
                    }
                    self.workload_diagnostics.record_payload_transaction_phase(
                        PayloadTransactionPhase::ApplicationPersist,
                        started.elapsed(),
                        application_event_count,
                    );
                    self.retained_payload_transaction
                        .record_persisted(trace_id, next_retained_bytes);
                }
                PreparedPayloadSegment::SemanticOnly {
                    segment,
                    semantic_actions,
                    application_events,
                } => {
                    let semantic_action_count = semantic_actions.actions().len();
                    if semantic_action_count != 0 || !semantic_actions.links().is_empty() {
                        session
                            .persist_semantic_actions(semantic_actions)
                            .map_err(recording_error_to_control)?;
                    }
                    let application_event_count = application_events.len();
                    for prepared_event in application_events {
                        session
                            .persist_event(prepared_event.event, prepared_event.semantic_actions)
                            .map_err(recording_error_to_control)?;
                    }
                    self.log_payload_diagnostic(format_args!(
                        "payload_persist semantic_only trace_id={} process_id={} stream={} captured_bytes={} semantic_actions={} application_events={} operation_id={}",
                        segment.trace_id,
                        segment.process.get(),
                        segment.protocol_hint.as_deref().unwrap_or("unknown"),
                        segment.captured_size,
                        semantic_action_count,
                        application_event_count,
                        segment.operation_id
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn observe_payload_semantics(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> SemanticActionBatch {
        let observation = if retain_evidence {
            self.semantic_actions
                .observe_payload_segment_with_diagnostics(segment)
        } else {
            self.semantic_actions
                .observe_unretained_payload_segment_with_diagnostics(segment)
        };
        self.mcp_stdio_diagnostics
            .extend(observation.mcp_stdio_diagnostics);
        let output = observation.output;
        let mut batch = SemanticActionBatch::from_action_output(
            output.actions,
            output.links,
            output.file_observation_paths,
            output.file_path_sets,
            output.llm_request_contents,
            output.llm_request_lineages,
            output.mcp_jsonrpc_contents,
            output.payload_segments,
        );
        if !retain_evidence {
            for action in batch.actions_mut() {
                action
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
        }
        batch
    }
}
