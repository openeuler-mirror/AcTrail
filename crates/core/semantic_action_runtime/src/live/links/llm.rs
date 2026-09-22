use std::collections::BTreeMap;

use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkOrigin,
    SemanticActionLinkRole, attr_keys as attrs,
};

/// Projects direct children already selected by the LLM correlator.
///
/// The call attributes are the correlation result. Link projection therefore
/// needs no cross-batch action cache or reverse-owner index: every call update
/// costs at most three attribute lookups and emits at most three links.
#[derive(Default)]
pub(super) struct LlmExchangeLinkProjector;

impl LlmExchangeLinkProjector {
    pub(super) fn observe_action(&self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        if action.kind != SemanticActionKind::LlmCall {
            return Vec::new();
        }

        let mut links = Vec::with_capacity(3);
        if let Some(request_action_id) = action.attributes.get(attrs::llm_call::REQUEST_ACTION_ID) {
            links.push(Self::direct_link(
                action,
                request_action_id,
                SemanticActionLinkRole::LlmCallRequest,
            ));
        }
        if let Some(response_action_id) = action.attributes.get(attrs::llm_call::RESPONSE_ACTION_ID)
        {
            links.push(Self::direct_link(
                action,
                response_action_id,
                SemanticActionLinkRole::LlmCallResponse,
            ));
        }
        if let Some(http_response_id) = action
            .attributes
            .get(attrs::llm_call::HTTP_RESPONSE_ACTION_ID)
        {
            links.push(Self::direct_link(
                action,
                http_response_id,
                SemanticActionLinkRole::LlmCallHttpResponse,
            ));
        }
        links
    }

    fn direct_link(
        call: &SemanticAction,
        child_action_id: &str,
        role: SemanticActionLinkRole,
    ) -> SemanticActionLink {
        SemanticActionLink {
            trace_id: call.trace_id,
            parent_action_id: call.action_id.clone(),
            child_action_id: child_action_id.to_string(),
            role,
            origin: SemanticActionLinkOrigin::Observed,
            valid: true,
            evidence: call.evidence.clone(),
            attributes: BTreeMap::new(),
        }
    }
}
