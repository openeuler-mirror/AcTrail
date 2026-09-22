use super::*;
use semantic_action::SemanticEvidenceKind;

pub(super) struct PreparedApplicationEvent {
    pub(super) event: DomainEvent,
    pub(super) semantic_actions: SemanticActionBatch,
}

pub(super) enum PreparedPayloadSegment {
    Retained {
        stored_segment: PayloadSegment,
        semantic_actions: SemanticActionBatch,
        application_events: Vec<PreparedApplicationEvent>,
        retained_body_bytes: u64,
    },
    SemanticOnly {
        diagnostic: PayloadDiagnosticMetadata,
        semantic_actions: SemanticActionBatch,
        application_events: Vec<PreparedApplicationEvent>,
    },
}

impl PreparedPayloadSegment {
    pub(super) fn has_durable_records(&self) -> bool {
        match self {
            Self::Retained { .. } => true,
            Self::SemanticOnly {
                semantic_actions,
                application_events,
                ..
            } => semantic_actions.has_durable_records() || !application_events.is_empty(),
        }
    }
}

impl PayloadTransactionContext<'_> {
    pub(super) fn record_semantic_output(
        &mut self,
        export_batch: &mut SemanticActionBatch,
        semantic_actions: &SemanticActionBatch,
    ) {
        self.semantic_action_count += semantic_actions.action_views().count();
        self.semantic_link_count += semantic_actions.links().len();
        StorageAttachService::append_recognized_agent_processes(
            semantic_actions,
            &mut self.recognized_agents,
        );
        if self.live_export_enabled {
            export_batch.extend(semantic_actions.clone());
        }
    }

    pub(super) fn prepare_incomplete_http1_response(
        &mut self,
        segment: &PayloadSegment,
        sequence: u64,
        header_projected: bool,
    ) -> SemanticActionBatch {
        let mut output = self.semantic_actions.prepare_incomplete_http1_response(
            segment,
            sequence,
            header_projected,
        );
        self.llm_pipeline_diagnostics
            .append(&mut output.llm_pipeline_diagnostics);
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
        )
    }

    pub(super) fn finish_incomplete_payload(
        &mut self,
        segment: &PayloadSegment,
    ) -> SemanticActionBatch {
        let mut output = self.semantic_actions.finish_incomplete_payload(segment);
        self.llm_pipeline_diagnostics
            .append(&mut output.llm_pipeline_diagnostics);
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
        )
    }

    pub(super) fn finish_incomplete_http1_response(
        &mut self,
        segment: &PayloadSegment,
    ) -> SemanticActionBatch {
        let mut output = self
            .semantic_actions
            .finish_incomplete_http1_response(segment);
        self.llm_pipeline_diagnostics
            .append(&mut output.llm_pipeline_diagnostics);
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
        )
    }

    pub(super) fn finish_llm_transaction(
        &mut self,
        segment: &PayloadSegment,
    ) -> SemanticActionBatch {
        let mut output = self.semantic_actions.finish_payload_transaction(segment);
        self.llm_pipeline_diagnostics
            .append(&mut output.llm_pipeline_diagnostics);
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
        )
    }

    pub(super) fn observe_payload_gap(&mut self, segment: &PayloadSegment) -> SemanticActionBatch {
        let observation = self.semantic_actions.observe_payload_gap(segment);
        self.mcp_stdio_diagnostics
            .extend(observation.mcp_stdio_diagnostics);
        let mut output = observation.output;
        self.llm_pipeline_diagnostics
            .append(&mut output.llm_pipeline_diagnostics);
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
        )
    }

    pub(super) fn observe_semantic_actions_for_event(
        &mut self,
        event: &DomainEvent,
    ) -> SemanticActionBatch {
        let observation = self.semantic_actions.observe_event_with_diagnostics(event);
        self.mcp_stdio_diagnostics
            .extend(observation.mcp_stdio_diagnostics);
        let mut output = observation.output;
        self.llm_pipeline_diagnostics
            .append(&mut output.llm_pipeline_diagnostics);
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
        )
    }

    pub(super) fn persist_prepared_payload_segments(
        &mut self,
        session: &mut ObservedRecordWriteSession<'_>,
        prepared: Vec<PreparedPayloadSegment>,
    ) -> Result<(), RecordingError> {
        for prepared in prepared {
            match prepared {
                PreparedPayloadSegment::Retained {
                    stored_segment,
                    semantic_actions,
                    application_events,
                    retained_body_bytes,
                } => {
                    let semantic_action_count = semantic_actions.action_views().count();
                    let trace_id = stored_segment.trace_id;
                    let process_id = stored_segment.process.get();
                    let source_boundary = stored_segment.source_boundary;
                    let captured_size = stored_segment.captured_size;
                    let operation_id = stored_segment.operation_id;
                    let started = crate::services::workload_diagnostics::now();
                    session.persist_payload_segment(stored_segment, semantic_actions)?;
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
                            .persist_event(prepared_event.event, prepared_event.semantic_actions)?;
                    }
                    self.workload_diagnostics.record_payload_transaction_phase(
                        PayloadTransactionPhase::ApplicationPersist,
                        started.elapsed(),
                        application_event_count,
                    );
                }
                PreparedPayloadSegment::SemanticOnly {
                    diagnostic,
                    semantic_actions,
                    application_events,
                } => {
                    let semantic_action_count = semantic_actions.action_views().count();
                    if semantic_actions.has_durable_records() {
                        session.persist_semantic_actions(semantic_actions)?;
                    }
                    let application_event_count = application_events.len();
                    for prepared_event in application_events {
                        session
                            .persist_event(prepared_event.event, prepared_event.semantic_actions)?;
                    }
                    self.log_payload_diagnostic(format_args!(
                        "payload_persist semantic_only trace_id={} process_id={} stream={} captured_bytes={} semantic_actions={} application_events={} operation_id={}",
                        diagnostic.trace_id,
                        diagnostic.process.get(),
                        diagnostic.protocol_hint.as_deref().unwrap_or("unknown"),
                        diagnostic.captured_size,
                        semantic_action_count,
                        application_event_count,
                        diagnostic.operation_id
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
        let mut output = observation.output;
        self.llm_pipeline_diagnostics
            .append(&mut output.llm_pipeline_diagnostics);
        let mut batch = SemanticActionBatch::from_action_output(
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
        );
        if !retain_evidence {
            batch.retain_evidence(|evidence| evidence.kind == SemanticEvidenceKind::Event);
        }
        batch
    }
}
