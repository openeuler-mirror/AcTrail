use std::collections::BTreeMap;

use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole,
};

use crate::live::actions::append_missing_evidence;
use crate::live::http_exchange::{HttpResponseMatch, MatchedHttpRequest};

/// Converts exact correlation proposals into links without retaining actions.
///
/// Request/response ownership is decided by the HTTP/LLM correlator while it
/// has both sides of an open exchange. Re-discovering that relationship from
/// all actions observed during a trace is both slower and less reliable.
#[derive(Default)]
pub(super) struct HttpMessageLinkProjector;

impl HttpMessageLinkProjector {
    pub(super) fn observe_exact_exchange_link(
        &self,
        exchange: &HttpResponseMatch,
    ) -> SemanticActionLink {
        SemanticActionLink {
            trace_id: exchange.response.trace_id,
            parent_action_id: exchange.request.action_id.clone(),
            child_action_id: exchange.response.action_id.clone(),
            role: SemanticActionLinkRole::HttpRequestHttpResponse,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence: Vec::new(),
            attributes: BTreeMap::new(),
        }
    }

    pub(super) fn observe_exact_request_link(
        &self,
        llm_request: &SemanticAction,
        http_request: &MatchedHttpRequest,
    ) -> Option<SemanticActionLink> {
        if llm_request.kind != SemanticActionKind::LlmRequest {
            return None;
        }
        let mut evidence = http_request.evidence.clone();
        append_missing_evidence(&mut evidence, &llm_request.evidence);
        Some(SemanticActionLink {
            trace_id: llm_request.trace_id,
            parent_action_id: llm_request.action_id.clone(),
            child_action_id: http_request.action_id.clone(),
            role: SemanticActionLinkRole::LlmRequestHttpMessage,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence,
            attributes: BTreeMap::new(),
        })
    }

    pub(super) fn observe_exact_response_link(
        &self,
        llm_response: &SemanticAction,
        http_response: &SemanticAction,
    ) -> Option<SemanticActionLink> {
        if llm_response.kind != SemanticActionKind::LlmResponse
            || http_response.kind != SemanticActionKind::HttpMessage
            || llm_response.trace_id != http_response.trace_id
            || llm_response.process != http_response.process
            || http_response
                .attributes
                .get("direction")
                .map(String::as_str)
                != Some("inbound")
        {
            return None;
        }
        let mut evidence = http_response.evidence.clone();
        append_missing_evidence(&mut evidence, &llm_response.evidence);
        Some(SemanticActionLink {
            trace_id: llm_response.trace_id,
            parent_action_id: llm_response.action_id.clone(),
            child_action_id: http_response.action_id.clone(),
            role: SemanticActionLinkRole::LlmResponseHttpMessage,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence,
            attributes: BTreeMap::new(),
        })
    }
}
