//! Exact link proposals emitted beside projected semantic actions.

use semantic_action::SemanticAction;

use crate::live::MatchedHttpRequest;

pub(crate) struct LlmHttpRequestLink {
    pub(crate) llm_request: SemanticAction,
    pub(crate) http_request: MatchedHttpRequest,
}

pub(crate) struct LlmHttpResponseLink {
    pub(crate) llm_response: SemanticAction,
    pub(crate) http_response: SemanticAction,
}

pub(crate) struct LlmContextCompactLink {
    pub(crate) compact: SemanticAction,
    pub(crate) implementation_call: SemanticAction,
}
