use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole,
};

use super::shared::{ActionLinkKey, SemanticActionKey};

#[derive(Default)]
pub(super) struct LlmExchangeLinkProjector {
    requests: BTreeMap<SemanticActionKey, SemanticAction>,
    responses: BTreeMap<SemanticActionKey, SemanticAction>,
    emitted_links: BTreeSet<ActionLinkKey>,
}

impl LlmExchangeLinkProjector {
    pub(super) fn observe_action(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        match action.kind {
            SemanticActionKind::LlmRequest => {
                self.requests
                    .insert(SemanticActionKey::from(action), action.clone());
                Vec::new()
            }
            SemanticActionKind::LlmResponse => {
                self.responses
                    .insert(SemanticActionKey::from(action), action.clone());
                self.latest_request_for_response(action)
                    .and_then(|request| self.link(&request, action))
                    .into_iter()
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.requests.retain(|key, _| key.trace_id != trace_id);
        self.responses.retain(|key, _| key.trace_id != trace_id);
        self.emitted_links.retain(|key| key.trace_id != trace_id);
    }

    fn latest_request_for_response(&self, response: &SemanticAction) -> Option<SemanticAction> {
        self.requests
            .values()
            .filter(|request| llm_actions_share_stream(request, response))
            .filter(|request| request.start_time <= response.start_time)
            .max_by_key(|request| (request.start_time, request.action_id.clone()))
            .cloned()
    }

    fn link(
        &mut self,
        request: &SemanticAction,
        response: &SemanticAction,
    ) -> Option<SemanticActionLink> {
        if !llm_actions_share_stream(request, response) || request.start_time > response.start_time
        {
            return None;
        }
        let key = ActionLinkKey {
            trace_id: request.trace_id,
            parent_action_id: request.action_id.clone(),
            child_action_id: response.action_id.clone(),
            role: SemanticActionLinkRole::LlmRequestLlmResponse,
        };
        if !self.emitted_links.insert(key) {
            return None;
        }
        Some(SemanticActionLink {
            trace_id: request.trace_id,
            parent_action_id: request.action_id.clone(),
            child_action_id: response.action_id.clone(),
            role: SemanticActionLinkRole::LlmRequestLlmResponse,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence: response.evidence.clone(),
            attributes: BTreeMap::new(),
        })
    }
}

fn llm_actions_share_stream(request: &SemanticAction, response: &SemanticAction) -> bool {
    request.kind == SemanticActionKind::LlmRequest
        && response.kind == SemanticActionKind::LlmResponse
        && request.trace_id == response.trace_id
        && request.process == response.process
        && request.attributes.get("payload.stream_key")
            == response.attributes.get("payload.stream_key")
        && http_stream_ids_are_compatible(request, response)
}

fn http_stream_ids_are_compatible(request: &SemanticAction, response: &SemanticAction) -> bool {
    match (
        request.attributes.get("http.request.stream_id"),
        response.attributes.get("http.response.stream_id"),
    ) {
        (Some(request_stream_id), Some(response_stream_id)) => {
            request_stream_id == response_stream_id
        }
        (None, None) => true,
        _ => false,
    }
}
