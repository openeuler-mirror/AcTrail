//! Unified native payload and lifecycle boundary.
use super::output::ActionBatch;
use super::pipeline::LiveLlmProjector;
use model_core::ids::TraceId;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use semantic_action::{SemanticAction, SemanticEvidenceKind};
use std::time::SystemTime;
pub(crate) enum PipelineEvent<'a> {
    PayloadSegment(&'a PayloadSegment),
    PayloadGap(&'a PayloadSegment),
    FinishIncompletePayload(&'a PayloadSegment),
    FinishPayloadTransaction(&'a PayloadSegment),
    ForgetPayloadStream(&'a PayloadStreamIdentity),
    FinalizePayloadStream {
        identity: &'a PayloadStreamIdentity,
        finished_at: SystemTime,
    },
    HttpObservation(&'a SemanticAction),
    ForgetTrace(TraceId),
    FinalizeTrace {
        trace_id: TraceId,
        finished_at: SystemTime,
    },
}
#[derive(Default)]
pub(crate) struct PipelineAdvance {
    pub(crate) output: ActionBatch,
}
impl LiveLlmProjector {
    pub(crate) fn advance(&mut self, event: PipelineEvent<'_>) -> PipelineAdvance {
        let mut output = match event {
            PipelineEvent::PayloadSegment(segment) => self.observe_payload_segment(segment),
            PipelineEvent::PayloadGap(segment) => self.observe_payload_gap(segment),
            PipelineEvent::FinishIncompletePayload(segment) => {
                self.finish_incomplete_payload(segment)
            }
            PipelineEvent::FinishPayloadTransaction(segment) => {
                self.finish_payload_transaction(segment)
            }
            PipelineEvent::ForgetPayloadStream(identity) => {
                self.forget_payload_stream(identity);
                ActionBatch::default()
            }
            PipelineEvent::FinalizePayloadStream {
                identity,
                finished_at,
            } => self.finalize_payload_stream(identity, finished_at),
            PipelineEvent::HttpObservation(action) => {
                let mut output = self.http1.merge_http_observation(action);
                output.extend(self.http2.merge_http_observation(action));
                output
            }
            PipelineEvent::ForgetTrace(trace_id) => {
                self.forget_trace(trace_id);
                ActionBatch::default()
            }
            PipelineEvent::FinalizeTrace {
                trace_id,
                finished_at,
            } => self.finalize_trace(trace_id, finished_at),
        };
        // Apply the same durable-evidence policy to payloads, later observations,
        // and every finalization path, including cached association proposals.
        let raw_payload = self.config.l4_payload.enabled;
        let assembled_payload = raw_payload || self.config.l0_llm_call.retain_assembled_payload();
        output.retain_evidence(|evidence| match evidence.kind {
            SemanticEvidenceKind::Event => true,
            SemanticEvidenceKind::PayloadSegment => raw_payload,
            SemanticEvidenceKind::PayloadAggregate => assembled_payload,
        });
        PipelineAdvance { output }
    }
}
