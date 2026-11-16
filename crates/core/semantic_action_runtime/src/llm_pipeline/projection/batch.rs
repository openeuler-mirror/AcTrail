//! Internal projection updates accumulated before facade export.

use model_core::diagnostics::LlmPipelineDiagnostic;
use model_core::payload::PayloadSegment;
use semantic_action::{LlmRequestContentWrite, LlmRequestLineageWrite, SemanticAction};

use super::compact::{CompletedResponse, LlmRequestObservation, LlmResponseObservation};
use super::links::{LlmContextCompactLink, LlmHttpRequestLink, LlmHttpResponseLink};
use super::projector::{
    ProjectedLlmRequestHistory, ProjectedLlmToolResult, ProjectedProviderResponseId,
};

#[derive(Default)]
pub(crate) struct ProjectionBatch {
    pub(crate) http1_messages: Vec<crate::Http1MessageEvent>,
    pub(crate) http1_handled: bool,
    pub(crate) http2_frames: Vec<crate::Http2FrameObservation>,
    pub(crate) http2_handled: bool,
    pub(crate) payload_consumed_by_http: bool,
    pub(crate) payload_consumed_by_llm: bool,
    pub(crate) links: Vec<semantic_action::SemanticActionLink>,
    pub(crate) actions: Vec<SemanticAction>,
    pub(crate) updates: Vec<semantic_action::SemanticActionUpdate>,
    pub(crate) updated_actions: Vec<SemanticAction>,
    pub(crate) llm_request_contents: Vec<LlmRequestContentWrite>,
    pub(in crate::llm_pipeline) request_observations: Vec<LlmRequestObservation>,
    pub(in crate::llm_pipeline) response_observations: Vec<LlmResponseObservation>,
    pub(crate) llm_request_lineages: Vec<LlmRequestLineageWrite>,
    pub(crate) llm_tool_results: Vec<ProjectedLlmToolResult>,
    pub(crate) diagnostics: Vec<LlmPipelineDiagnostic>,
    pub(in crate::llm_pipeline) llm_request_histories: Vec<ProjectedLlmRequestHistory>,
    pub(in crate::llm_pipeline) provider_response_ids: Vec<ProjectedProviderResponseId>,
    pub(crate) payload_segments: Vec<PayloadSegment>,
    pub(crate) http_request_links: Vec<LlmHttpRequestLink>,
    pub(crate) http_response_links: Vec<LlmHttpResponseLink>,
    pub(crate) context_compact_links: Vec<LlmContextCompactLink>,
}

/// One owner-resolved association consumed during the current projection batch.
pub(in crate::llm_pipeline) struct ProjectedResponseBinding {
    pub(in crate::llm_pipeline) request: SemanticAction,
    pub(in crate::llm_pipeline) completion: Option<CompletedResponse>,
    pub(in crate::llm_pipeline) provider_response_id: Option<String>,
    pub(in crate::llm_pipeline) observed_at: std::time::SystemTime,
    pub(in crate::llm_pipeline) failed: bool,
}

impl ProjectionBatch {
    pub(in crate::llm_pipeline) fn retain_evidence(
        &mut self,
        retain: impl Fn(&semantic_action::SemanticEvidence) -> bool,
    ) {
        for action in self.actions.iter_mut().chain(&mut self.updated_actions) {
            action.evidence.retain(&retain);
        }
        for update in &mut self.updates {
            update.evidence.retain(&retain);
        }
        for link in &mut self.links {
            link.evidence.retain(&retain);
        }
        for proposal in &mut self.http_request_links {
            proposal.llm_request.evidence.retain(&retain);
            proposal.http_request.evidence.retain(&retain);
        }
        for proposal in &mut self.http_response_links {
            proposal.llm_response.evidence.retain(&retain);
            proposal.http_response.evidence.retain(&retain);
        }
        for proposal in &mut self.context_compact_links {
            proposal.compact.evidence.retain(&retain);
            proposal.implementation_call.evidence.retain(&retain);
        }
    }

    pub(in crate::llm_pipeline) fn extend(&mut self, other: Self) {
        self.http1_messages.extend(other.http1_messages);
        self.http1_handled |= other.http1_handled;
        self.http2_frames.extend(other.http2_frames);
        self.http2_handled |= other.http2_handled;
        self.links.extend(other.links);
        self.actions.extend(other.actions);
        self.updates.extend(other.updates);
        self.updated_actions.extend(other.updated_actions);
        self.llm_request_contents.extend(other.llm_request_contents);
        self.request_observations.extend(other.request_observations);
        self.response_observations
            .extend(other.response_observations);
        self.llm_request_lineages.extend(other.llm_request_lineages);
        self.llm_tool_results.extend(other.llm_tool_results);
        self.diagnostics.extend(other.diagnostics);
        self.llm_request_histories
            .extend(other.llm_request_histories);
        self.provider_response_ids
            .extend(other.provider_response_ids);
        self.payload_segments.extend(other.payload_segments);
        self.http_request_links.extend(other.http_request_links);
        self.http_response_links.extend(other.http_response_links);
        self.context_compact_links
            .extend(other.context_compact_links);
    }
}
