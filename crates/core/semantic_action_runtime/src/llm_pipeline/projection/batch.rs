//! Internal projection updates accumulated before facade export.

use std::collections::{BTreeSet, HashSet};

use model_core::diagnostics::LlmPipelineDiagnostic;
use model_core::payload::PayloadSegment;
use semantic_action::{
    LlmRequestContentWrite, LlmRequestLineageWrite, SemanticAction, SemanticActionKind,
};

use super::links::{LlmHttpRequestLink, LlmHttpResponseLink};
use super::projector::{
    ProjectedLlmRequestHistory, ProjectedLlmToolResult, ProjectedProviderResponseId,
};

#[derive(Default)]
pub(crate) struct ProjectionBatch {
    pub(crate) actions: Vec<SemanticAction>,
    pub(crate) llm_request_contents: Vec<LlmRequestContentWrite>,
    pub(crate) llm_request_lineages: Vec<LlmRequestLineageWrite>,
    pub(crate) llm_tool_results: Vec<ProjectedLlmToolResult>,
    pub(crate) diagnostics: Vec<LlmPipelineDiagnostic>,
    pub(in crate::llm_pipeline) llm_request_histories: Vec<ProjectedLlmRequestHistory>,
    pub(in crate::llm_pipeline) provider_response_ids: Vec<ProjectedProviderResponseId>,
    pub(in crate::llm_pipeline) non_reusable_response_ids: BTreeSet<String>,
    pub(in crate::llm_pipeline) closed_response_ids: HashSet<String>,
    pub(crate) payload_segments: Vec<PayloadSegment>,
    pub(crate) http_request_links: Vec<LlmHttpRequestLink>,
    pub(crate) http_response_links: Vec<LlmHttpResponseLink>,
}

impl ProjectionBatch {
    pub(in crate::llm_pipeline) fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.llm_request_contents.extend(other.llm_request_contents);
        self.llm_request_lineages.extend(other.llm_request_lineages);
        self.llm_tool_results.extend(other.llm_tool_results);
        self.diagnostics.extend(other.diagnostics);
        self.llm_request_histories
            .extend(other.llm_request_histories);
        self.provider_response_ids
            .extend(other.provider_response_ids);
        self.non_reusable_response_ids
            .extend(other.non_reusable_response_ids);
        self.closed_response_ids.extend(other.closed_response_ids);
        self.payload_segments.extend(other.payload_segments);
        self.http_request_links.extend(other.http_request_links);
        self.http_response_links.extend(other.http_response_links);
    }

    pub(in crate::llm_pipeline) fn mark_closed_response_actions(
        &mut self,
        actions: &[SemanticAction],
    ) {
        self.closed_response_ids.extend(
            actions
                .iter()
                .filter(|action| action.kind == SemanticActionKind::LlmResponse)
                .map(|action| action.action_id.clone()),
        );
    }
}
