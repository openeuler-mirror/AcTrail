use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole, attr_keys as attrs,
};

use crate::live::actions::{ATTR_LINK_VALID, LINK_VALID_FALSE};

use super::shared::{ActionLinkKey, SemanticActionKey};

#[derive(Default)]
pub(super) struct LlmExchangeLinkProjector {
    calls: BTreeMap<SemanticActionKey, SemanticAction>,
    requests: BTreeMap<SemanticActionKey, SemanticAction>,
    responses: BTreeMap<SemanticActionKey, SemanticAction>,
    emitted_links: BTreeSet<ActionLinkKey>,
}

impl LlmExchangeLinkProjector {
    pub(super) fn observe_action(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        match action.kind {
            SemanticActionKind::LlmCall => {
                let previous = self
                    .calls
                    .insert(SemanticActionKey::from(action), action.clone());
                let mut links = Vec::new();
                if let Some(previous) = previous.as_ref() {
                    links.extend(self.invalidate_superseded_child_links(previous, action));
                }
                if let Some(request) = self.call_request(action) {
                    links.extend(self.link(
                        action,
                        &request,
                        SemanticActionLinkRole::LlmCallRequest,
                    ));
                }
                if let Some(response) = self.call_response(action) {
                    links.extend(self.link(
                        action,
                        &response,
                        SemanticActionLinkRole::LlmCallResponse,
                    ));
                }
                links
            }
            SemanticActionKind::LlmRequest => {
                self.requests
                    .insert(SemanticActionKey::from(action), action.clone());
                self.calls
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                    .iter()
                    .filter_map(|call| {
                        call_references_action(call, attrs::llm_call::REQUEST_ACTION_ID, action)
                            .then(|| {
                                self.link(call, action, SemanticActionLinkRole::LlmCallRequest)
                            })
                            .flatten()
                    })
                    .collect()
            }
            SemanticActionKind::LlmResponse => {
                self.responses
                    .insert(SemanticActionKey::from(action), action.clone());
                self.calls
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                    .iter()
                    .filter_map(|call| {
                        call_references_action(call, attrs::llm_call::RESPONSE_ACTION_ID, action)
                            .then(|| {
                                self.link(call, action, SemanticActionLinkRole::LlmCallResponse)
                            })
                            .flatten()
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.calls.retain(|key, _| key.trace_id != trace_id);
        self.requests.retain(|key, _| key.trace_id != trace_id);
        self.responses.retain(|key, _| key.trace_id != trace_id);
        self.emitted_links.retain(|key| key.trace_id != trace_id);
    }

    fn call_request(&self, call: &SemanticAction) -> Option<SemanticAction> {
        let action_id = call.attributes.get(attrs::llm_call::REQUEST_ACTION_ID)?;
        self.requests
            .values()
            .find(|request| request.action_id == *action_id)
            .cloned()
    }

    fn call_response(&self, call: &SemanticAction) -> Option<SemanticAction> {
        let action_id = call.attributes.get(attrs::llm_call::RESPONSE_ACTION_ID)?;
        self.responses
            .values()
            .find(|response| response.action_id == *action_id)
            .cloned()
    }

    fn link(
        &mut self,
        call: &SemanticAction,
        child: &SemanticAction,
        role: SemanticActionLinkRole,
    ) -> Option<SemanticActionLink> {
        if !call_references_role(call, child, role) {
            return None;
        }
        let key = ActionLinkKey {
            trace_id: call.trace_id,
            parent_action_id: call.action_id.clone(),
            child_action_id: child.action_id.clone(),
            role,
        };
        if !self.emitted_links.insert(key) {
            return None;
        }
        Some(SemanticActionLink {
            trace_id: call.trace_id,
            parent_action_id: call.action_id.clone(),
            child_action_id: child.action_id.clone(),
            role,
            confidence: SemanticActionLinkConfidence::Observed,
            evidence: child.evidence.clone(),
            attributes: BTreeMap::new(),
        })
    }

    fn invalidate_superseded_child_links(
        &mut self,
        previous: &SemanticAction,
        current: &SemanticAction,
    ) -> Vec<SemanticActionLink> {
        [
            (
                attrs::llm_call::REQUEST_ACTION_ID,
                SemanticActionLinkRole::LlmCallRequest,
            ),
            (
                attrs::llm_call::RESPONSE_ACTION_ID,
                SemanticActionLinkRole::LlmCallResponse,
            ),
        ]
        .into_iter()
        .filter_map(|(attr, role)| {
            let old_child = previous.attributes.get(attr)?;
            let new_child = current.attributes.get(attr)?;
            (old_child != new_child).then(|| self.invalidate_link(current, old_child, role))
        })
        .collect()
    }

    fn invalidate_link(
        &mut self,
        call: &SemanticAction,
        child_action_id: &str,
        role: SemanticActionLinkRole,
    ) -> SemanticActionLink {
        let key = ActionLinkKey {
            trace_id: call.trace_id,
            parent_action_id: call.action_id.clone(),
            child_action_id: child_action_id.to_string(),
            role,
        };
        self.emitted_links.remove(&key);
        SemanticActionLink {
            trace_id: call.trace_id,
            parent_action_id: call.action_id.clone(),
            child_action_id: child_action_id.to_string(),
            role,
            confidence: SemanticActionLinkConfidence::Derived,
            evidence: call.evidence.clone(),
            attributes: BTreeMap::from([(
                ATTR_LINK_VALID.to_string(),
                LINK_VALID_FALSE.to_string(),
            )]),
        }
    }
}

fn call_references_role(
    call: &SemanticAction,
    child: &SemanticAction,
    role: SemanticActionLinkRole,
) -> bool {
    match role {
        SemanticActionLinkRole::LlmCallRequest => {
            call_references_action(call, attrs::llm_call::REQUEST_ACTION_ID, child)
                && child.kind == SemanticActionKind::LlmRequest
        }
        SemanticActionLinkRole::LlmCallResponse => {
            call_references_action(call, attrs::llm_call::RESPONSE_ACTION_ID, child)
                && child.kind == SemanticActionKind::LlmResponse
        }
        _ => false,
    }
}

fn call_references_action(call: &SemanticAction, attr: &str, child: &SemanticAction) -> bool {
    call.kind == SemanticActionKind::LlmCall
        && call.trace_id == child.trace_id
        && call.process == child.process
        && call
            .attributes
            .get(attr)
            .is_some_and(|action_id| action_id == &child.action_id)
}
