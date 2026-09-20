//! Payload-driven semantic projection and stream completion.

use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use semantic_action::SemanticEvidenceKind;
use std::time::SystemTime;

use crate::live::http_exchange::DamagedHttp1RequestOutcome;
use crate::llm_pipeline::PipelineEvent;

use super::{LiveSemanticActionObservation, LiveSemanticActionOutput, LiveSemanticActionRuntime};

impl LiveSemanticActionRuntime {
    pub fn observe_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        self.observe_payload_segment_with_diagnostics(segment)
            .output
    }

    pub fn observe_payload_segment_with_diagnostics(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionObservation {
        self.observe_payload_segment_with_evidence(segment, true)
    }

    /// Projects unretained stdio through MCP without payload evidence.
    pub fn observe_unretained_mcp_stdio_payload_segment(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        self.observe_unretained_mcp_stdio_payload_segment_with_diagnostics(segment)
            .output
    }

    pub fn observe_unretained_mcp_stdio_payload_segment_with_diagnostics(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionObservation {
        assert!(
            segment.source_boundary == model_core::payload::PayloadSourceBoundary::Stdio,
            "unretained MCP projection requires a stdio payload segment"
        );
        self.observe_payload_segment_with_evidence(segment, false)
    }

    pub fn observe_unretained_payload_segment_with_diagnostics(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionObservation {
        self.observe_payload_segment_with_evidence(segment, false)
    }

    fn observe_payload_segment_with_evidence(
        &mut self,
        segment: &PayloadSegment,
        retain_evidence: bool,
    ) -> LiveSemanticActionObservation {
        if !self.projection_enabled {
            return LiveSemanticActionObservation {
                output: LiveSemanticActionOutput::default(),
                mcp_stdio_diagnostics: Vec::new(),
            };
        }
        if retain_evidence {
            self.agent.observe_payload_segment(segment);
        }
        let mut llm_output = self
            .llm
            .advance(PipelineEvent::PayloadSegment(segment))
            .output;
        if !retain_evidence {
            for action in llm_output
                .actions
                .iter_mut()
                .chain(llm_output.updated_actions.iter_mut())
            {
                action
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
            for update in &mut llm_output.updates {
                update
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
        }
        let projected_llm = self.observe_llm_batch(llm_output);
        let (mcp_output, mcp_stdio_diagnostics) =
            self.mcp.observe_payload_segment(segment, retain_evidence);
        let mcp_actions = mcp_output.actions;
        let mut output = if projected_llm.actions.is_empty()
            && projected_llm.updated_actions.is_empty()
            && mcp_actions.is_empty()
            && mcp_output.updated_actions.is_empty()
        {
            LiveSemanticActionOutput::default()
        } else {
            self.file_access.observe_boundary(
                segment.trace_id,
                &segment.process,
                segment.observed_at,
            )
        };
        output.extend(projected_llm);
        for action in &mcp_actions {
            output.extend(self.command.observe_mcp_tool_call(action));
        }
        for action in &mcp_output.updated_actions {
            output.extend(self.command.observe_mcp_tool_call(action));
        }
        output.actions.extend(mcp_actions);
        output.updates.extend(mcp_output.updates);
        output.updated_actions.extend(mcp_output.updated_actions);
        output.links.extend(mcp_output.links);
        output.mcp_jsonrpc_contents.extend(mcp_output.contents);
        output.payload_segments.extend(mcp_output.payload_segments);
        output.links.extend(
            self.links
                .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
        );
        if !retain_evidence {
            for action in output
                .actions
                .iter_mut()
                .chain(output.updated_actions.iter_mut())
            {
                action
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
            for update in &mut output.updates {
                update
                    .evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
            for link in &mut output.links {
                link.evidence
                    .retain(|evidence| evidence.kind == SemanticEvidenceKind::Event);
            }
        }
        LiveSemanticActionObservation {
            output,
            mcp_stdio_diagnostics,
        }
    }

    pub fn should_project_dropped_stdio_payload(&self, segment: &PayloadSegment) -> bool {
        self.projection_enabled && self.mcp.should_project_stdio_payload(segment)
    }

    pub fn observe_payload_gap(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionObservation {
        if !self.projection_enabled {
            return LiveSemanticActionObservation {
                output: LiveSemanticActionOutput::default(),
                mcp_stdio_diagnostics: Vec::new(),
            };
        }
        self.http_exchange.quarantine_payload_stream(segment);
        let llm_output = self.llm.advance(PipelineEvent::PayloadGap(segment)).output;
        let mut output = self.observe_llm_batch(llm_output);
        output.links.extend(
            self.links
                .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
        );
        LiveSemanticActionObservation {
            output,
            mcp_stdio_diagnostics: Vec::new(),
        }
    }

    pub fn prepare_incomplete_payload(&mut self, segment: &PayloadSegment) {
        if !self.projection_enabled {
            return;
        }
        self.http_exchange.quarantine_payload_stream(segment);
    }

    pub fn prepare_incomplete_http1_request(
        &mut self,
        segment: &PayloadSegment,
        sequence: u64,
        header_projected: bool,
    ) {
        if !self.projection_enabled {
            return;
        }
        match self
            .http_exchange
            .observe_damaged_http1_request(segment, sequence, header_projected)
        {
            DamagedHttp1RequestOutcome::Tombstoned => {}
            DamagedHttp1RequestOutcome::MissingPending
                if self
                    .llm
                    .advance(PipelineEvent::LocalizeIncompleteHttp1Request { segment, sequence })
                    .localized => {}
            DamagedHttp1RequestOutcome::MissingPending => {
                self.http_exchange.quarantine_payload_stream(segment);
                self.llm
                    .advance(PipelineEvent::ForgetPayloadAssociations(segment));
            }
            DamagedHttp1RequestOutcome::Unsafe => {
                self.llm
                    .advance(PipelineEvent::ForgetPayloadAssociations(segment));
            }
        }
    }

    pub fn prepare_incomplete_http1_response(
        &mut self,
        segment: &PayloadSegment,
        sequence: u64,
        header_projected: bool,
    ) -> LiveSemanticActionOutput {
        if !self.projection_enabled {
            return LiveSemanticActionOutput::default();
        }
        let request = (!header_projected)
            .then(|| self.http_exchange.observe_damaged_http1_response(segment))
            .flatten();
        let llm_output = self
            .llm
            .advance(PipelineEvent::PrepareIncompleteHttp1Response {
                segment,
                sequence,
                request,
            })
            .output;
        let mut output = self.observe_llm_batch(llm_output);
        output.links.extend(
            self.links
                .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
        );
        output
    }

    pub fn finish_incomplete_payload(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        if !self.projection_enabled {
            return LiveSemanticActionOutput::default();
        }
        let llm_output = self
            .llm
            .advance(PipelineEvent::FinishIncompletePayload(segment))
            .output;
        let mut output = self.observe_llm_batch(llm_output);
        output.links.extend(
            self.links
                .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
        );
        output
    }

    pub fn finish_incomplete_http1_response(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        if !self.projection_enabled {
            return LiveSemanticActionOutput::default();
        }
        let llm_output = self
            .llm
            .advance(PipelineEvent::FinishIncompleteHttp1Response(segment))
            .output;
        let mut output = self.observe_llm_batch(llm_output);
        output.links.extend(
            self.links
                .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
        );
        output
    }

    pub fn finish_payload_transaction(
        &mut self,
        segment: &PayloadSegment,
    ) -> LiveSemanticActionOutput {
        if !self.projection_enabled {
            return LiveSemanticActionOutput::default();
        }
        let llm_output = self
            .llm
            .advance(PipelineEvent::FinishPayloadTransaction(segment))
            .output;
        let mut output = self.observe_llm_batch(llm_output);
        output.links.extend(
            self.links
                .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
        );
        output
    }

    pub fn forget_payload_stream(&mut self, identity: &PayloadStreamIdentity) {
        self.http_exchange.forget_payload_stream(identity);
        self.llm
            .advance(PipelineEvent::ForgetPayloadStream(identity));
    }

    pub fn finalize_payload_stream(
        &mut self,
        identity: &PayloadStreamIdentity,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        if !self.projection_enabled {
            return LiveSemanticActionOutput::default();
        }
        let llm_output = self
            .llm
            .advance(PipelineEvent::FinalizePayloadStream {
                identity,
                finished_at,
            })
            .output;
        let mut output = self.observe_llm_batch(llm_output);
        output.links.extend(
            self.links
                .observe_actions(output.actions.iter().chain(output.updated_actions.iter())),
        );
        self.http_exchange.forget_payload_stream(identity);
        output
    }
}
