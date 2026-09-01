//! Unified data and lifecycle boundary accepted by the LLM pipeline.

use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use semantic_action::SemanticAction;

use crate::live::{HttpResponseMatch, MatchedHttpRequest};

use super::output::ActionBatch;
use super::pipeline::LiveLlmProjector;

pub(crate) enum PipelineEvent<'a> {
    PayloadSegment(&'a PayloadSegment),
    PayloadGap(&'a PayloadSegment),
    FinishIncompletePayload(&'a PayloadSegment),
    FinishPayloadTransaction(&'a PayloadSegment),
    ForgetPayloadAssociations(&'a PayloadSegment),
    ForgetPayloadStream(&'a PayloadStreamIdentity),
    FinalizePayloadStream {
        identity: &'a PayloadStreamIdentity,
        finished_at: SystemTime,
    },
    HttpExchange(&'a HttpResponseMatch),
    DamagedHttpResponse(&'a SemanticAction),
    UnmatchedHttpResponse(&'a SemanticAction),
    PrepareIncompleteHttp1Response {
        segment: &'a PayloadSegment,
        sequence: u64,
        request: Option<MatchedHttpRequest>,
    },
    LocalizeIncompleteHttp1Request {
        segment: &'a PayloadSegment,
        sequence: u64,
    },
    FinishIncompleteHttp1Response(&'a PayloadSegment),
    ForgetTrace(TraceId),
    FinalizeTrace {
        trace_id: TraceId,
        finished_at: SystemTime,
    },
}

#[derive(Default)]
pub(crate) struct PipelineAdvance {
    pub(crate) output: ActionBatch,
    pub(crate) localized: bool,
}

impl LiveLlmProjector {
    pub(crate) fn advance(&mut self, event: PipelineEvent<'_>) -> PipelineAdvance {
        match event {
            PipelineEvent::PayloadSegment(segment) => self.observe_payload_segment(segment).into(),
            PipelineEvent::PayloadGap(segment) => self.observe_payload_gap(segment).into(),
            PipelineEvent::FinishIncompletePayload(segment) => {
                self.finish_incomplete_payload(segment).into()
            }
            PipelineEvent::FinishPayloadTransaction(segment) => {
                self.finish_payload_transaction(segment).into()
            }
            PipelineEvent::ForgetPayloadAssociations(segment) => {
                self.forget_payload_associations(segment);
                PipelineAdvance::default()
            }
            PipelineEvent::ForgetPayloadStream(identity) => {
                self.forget_payload_stream(identity);
                PipelineAdvance::default()
            }
            PipelineEvent::FinalizePayloadStream {
                identity,
                finished_at,
            } => self.finalize_payload_stream(identity, finished_at).into(),
            PipelineEvent::HttpExchange(exchange) => {
                self.projection.observe_http_exchange(exchange).into()
            }
            PipelineEvent::DamagedHttpResponse(response) => self
                .projection
                .observe_damaged_http_response(response)
                .into(),
            PipelineEvent::UnmatchedHttpResponse(response) => self
                .projection
                .observe_unmatched_http_response(response)
                .into(),
            PipelineEvent::PrepareIncompleteHttp1Response {
                segment,
                sequence,
                request,
            } => self
                .projection
                .prepare_incomplete_http1_response(segment, sequence, request)
                .into(),
            PipelineEvent::LocalizeIncompleteHttp1Request { segment, sequence } => {
                PipelineAdvance {
                    localized: self
                        .projection
                        .localize_incomplete_http1_request(segment, sequence),
                    ..PipelineAdvance::default()
                }
            }
            PipelineEvent::FinishIncompleteHttp1Response(segment) => self
                .projection
                .finish_incomplete_http1_response(segment)
                .into(),
            PipelineEvent::ForgetTrace(trace_id) => {
                self.forget_trace(trace_id);
                PipelineAdvance::default()
            }
            PipelineEvent::FinalizeTrace {
                trace_id,
                finished_at,
            } => self.finalize_trace(trace_id, finished_at).into(),
        }
    }
}

impl From<ActionBatch> for PipelineAdvance {
    fn from(output: ActionBatch) -> Self {
        Self {
            output,
            localized: false,
        }
    }
}
