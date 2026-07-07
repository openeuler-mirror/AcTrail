//! Links for MCP stdio details under MCP tool calls.

use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole, attr_keys as attrs,
};

use super::shared::ActionLinkKey;

#[derive(Default)]
pub(super) struct McpLinkProjector {
    actions: BTreeMap<(TraceId, String), SemanticAction>,
    emitted_links: BTreeSet<ActionLinkKey>,
}

impl McpLinkProjector {
    pub(super) fn observe_action(&mut self, action: &SemanticAction) -> Vec<SemanticActionLink> {
        if !is_mcp_detail_action(action) {
            return Vec::new();
        }
        self.actions
            .insert((action.trace_id, action.action_id.clone()), action.clone());

        let mut links = Vec::new();
        match action.kind {
            SemanticActionKind::McpToolCall => {
                links.extend(self.link_existing_children_by_reference(
                    action,
                    SemanticActionKind::McpRequest,
                    attrs::mcp::TOOL_CALL_ACTION_ID,
                    SemanticActionLinkRole::McpToolCallRequest,
                ));
                links.extend(self.link_existing_child_by_id(
                    action,
                    &mcp_child_action_id(&action.action_id, "request"),
                    SemanticActionLinkRole::McpToolCallRequest,
                ));
                links.extend(self.link_existing_children_by_reference(
                    action,
                    SemanticActionKind::McpResponse,
                    attrs::mcp::TOOL_CALL_ACTION_ID,
                    SemanticActionLinkRole::McpToolCallResponse,
                ));
                links.extend(self.link_existing_child_by_id(
                    action,
                    &mcp_child_action_id(&action.action_id, "response"),
                    SemanticActionLinkRole::McpToolCallResponse,
                ));
            }
            SemanticActionKind::McpRequest => {
                links.extend(self.link_existing_parent_by_id(
                    &mcp_parent_action_id(&action.action_id, "request"),
                    action,
                    SemanticActionLinkRole::McpToolCallRequest,
                ));
                links.extend(self.link_existing_parent_from_attr(
                    action,
                    attrs::mcp::TOOL_CALL_ACTION_ID,
                    SemanticActionLinkRole::McpToolCallRequest,
                ));
                links.extend(self.link_existing_children_by_reference(
                    action,
                    SemanticActionKind::McpStdout,
                    attrs::mcp::REQUEST_ACTION_ID,
                    SemanticActionLinkRole::McpRequestStdout,
                ));
                links.extend(self.link_existing_child_by_id(
                    action,
                    &mcp_sibling_action_id(&action.action_id, "request", "stdout"),
                    SemanticActionLinkRole::McpRequestStdout,
                ));
                links.extend(self.link_existing_children_by_reference(
                    action,
                    SemanticActionKind::McpClientSend,
                    attrs::mcp::REQUEST_ACTION_ID,
                    SemanticActionLinkRole::McpRequestClientSend,
                ));
                links.extend(self.link_existing_child_by_id(
                    action,
                    &mcp_sibling_action_id(&action.action_id, "request", "client_send"),
                    SemanticActionLinkRole::McpRequestClientSend,
                ));
            }
            SemanticActionKind::McpStdout => {
                links.extend(self.link_existing_parent_from_attr(
                    action,
                    attrs::mcp::REQUEST_ACTION_ID,
                    SemanticActionLinkRole::McpRequestStdout,
                ));
                links.extend(self.link_existing_parent_by_id(
                    &mcp_sibling_action_id(&action.action_id, "stdout", "request"),
                    action,
                    SemanticActionLinkRole::McpRequestStdout,
                ));
            }
            SemanticActionKind::McpClientSend => {
                links.extend(self.link_existing_parent_from_attr(
                    action,
                    attrs::mcp::REQUEST_ACTION_ID,
                    SemanticActionLinkRole::McpRequestClientSend,
                ));
                links.extend(self.link_existing_parent_by_id(
                    &mcp_sibling_action_id(&action.action_id, "client_send", "request"),
                    action,
                    SemanticActionLinkRole::McpRequestClientSend,
                ));
            }
            SemanticActionKind::McpResponse => {
                links.extend(self.link_existing_parent_by_id(
                    &mcp_parent_action_id(&action.action_id, "response"),
                    action,
                    SemanticActionLinkRole::McpToolCallResponse,
                ));
                links.extend(self.link_existing_parent_from_attr(
                    action,
                    attrs::mcp::TOOL_CALL_ACTION_ID,
                    SemanticActionLinkRole::McpToolCallResponse,
                ));
                links.extend(self.link_existing_children_by_reference(
                    action,
                    SemanticActionKind::McpStdin,
                    attrs::mcp::RESPONSE_ACTION_ID,
                    SemanticActionLinkRole::McpResponseStdin,
                ));
                links.extend(self.link_existing_child_by_id(
                    action,
                    &mcp_sibling_action_id(&action.action_id, "response", "stdin"),
                    SemanticActionLinkRole::McpResponseStdin,
                ));
                links.extend(self.link_existing_children_by_reference(
                    action,
                    SemanticActionKind::McpClientReceive,
                    attrs::mcp::RESPONSE_ACTION_ID,
                    SemanticActionLinkRole::McpResponseClientReceive,
                ));
                links.extend(self.link_existing_child_by_id(
                    action,
                    &mcp_sibling_action_id(&action.action_id, "response", "client_receive"),
                    SemanticActionLinkRole::McpResponseClientReceive,
                ));
            }
            SemanticActionKind::McpStdin => {
                links.extend(self.link_existing_parent_from_attr(
                    action,
                    attrs::mcp::RESPONSE_ACTION_ID,
                    SemanticActionLinkRole::McpResponseStdin,
                ));
                links.extend(self.link_existing_parent_by_id(
                    &mcp_sibling_action_id(&action.action_id, "stdin", "response"),
                    action,
                    SemanticActionLinkRole::McpResponseStdin,
                ));
            }
            SemanticActionKind::McpClientReceive => {
                links.extend(self.link_existing_parent_from_attr(
                    action,
                    attrs::mcp::RESPONSE_ACTION_ID,
                    SemanticActionLinkRole::McpResponseClientReceive,
                ));
                links.extend(self.link_existing_parent_by_id(
                    &mcp_sibling_action_id(&action.action_id, "client_receive", "response"),
                    action,
                    SemanticActionLinkRole::McpResponseClientReceive,
                ));
            }
            _ => {}
        }
        links
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.actions
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.emitted_links.retain(|key| key.trace_id != trace_id);
    }

    fn link_existing_parent_by_id(
        &mut self,
        parent_action_id: &str,
        child: &SemanticAction,
        role: SemanticActionLinkRole,
    ) -> Option<SemanticActionLink> {
        let parent = self
            .actions
            .get(&(child.trace_id, parent_action_id.to_string()))
            .cloned()?;
        self.link(&parent, child, role)
    }

    fn link_existing_parent_from_attr(
        &mut self,
        child: &SemanticAction,
        attr: &str,
        role: SemanticActionLinkRole,
    ) -> Option<SemanticActionLink> {
        let parent_action_id = child.attributes.get(attr)?;
        let parent = self
            .actions
            .get(&(child.trace_id, parent_action_id.clone()))
            .cloned()?;
        self.link(&parent, child, role)
    }

    fn link_existing_child_by_id(
        &mut self,
        parent: &SemanticAction,
        child_action_id: &str,
        role: SemanticActionLinkRole,
    ) -> Option<SemanticActionLink> {
        let child = self
            .actions
            .get(&(parent.trace_id, child_action_id.to_string()))
            .cloned()?;
        self.link(parent, &child, role)
    }

    fn link_existing_children_by_reference(
        &mut self,
        parent: &SemanticAction,
        child_kind: SemanticActionKind,
        attr: &str,
        role: SemanticActionLinkRole,
    ) -> Vec<SemanticActionLink> {
        let children = self
            .actions
            .values()
            .filter(|child| {
                child.trace_id == parent.trace_id
                    && child.kind == child_kind
                    && child
                        .attributes
                        .get(attr)
                        .is_some_and(|action_id| action_id == &parent.action_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        children
            .into_iter()
            .filter_map(|child| self.link(parent, &child, role))
            .collect()
    }

    fn link(
        &mut self,
        parent: &SemanticAction,
        child: &SemanticAction,
        role: SemanticActionLinkRole,
    ) -> Option<SemanticActionLink> {
        if !valid_mcp_link(parent, child, role) {
            return None;
        }
        let key = ActionLinkKey {
            trace_id: parent.trace_id,
            parent_action_id: parent.action_id.clone(),
            child_action_id: child.action_id.clone(),
            role,
        };
        if !self.emitted_links.insert(key) {
            return None;
        }
        Some(SemanticActionLink {
            trace_id: parent.trace_id,
            parent_action_id: parent.action_id.clone(),
            child_action_id: child.action_id.clone(),
            role,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence: child.evidence.clone(),
            attributes: BTreeMap::new(),
        })
    }
}

fn is_mcp_detail_action(action: &SemanticAction) -> bool {
    matches!(
        action.kind,
        SemanticActionKind::McpToolCall
            | SemanticActionKind::McpRequest
            | SemanticActionKind::McpResponse
            | SemanticActionKind::McpClientSend
            | SemanticActionKind::McpClientReceive
            | SemanticActionKind::McpStdin
            | SemanticActionKind::McpStdout
    )
}

fn valid_mcp_link(
    parent: &SemanticAction,
    child: &SemanticAction,
    role: SemanticActionLinkRole,
) -> bool {
    parent.trace_id == child.trace_id
        && parent.process == child.process
        && match role {
            SemanticActionLinkRole::McpToolCallRequest => {
                parent.kind == SemanticActionKind::McpToolCall
                    && child.kind == SemanticActionKind::McpRequest
                    && (child_references_parent(child, attrs::mcp::TOOL_CALL_ACTION_ID, parent)
                        || child.action_id == mcp_child_action_id(&parent.action_id, "request"))
            }
            SemanticActionLinkRole::McpToolCallResponse => {
                parent.kind == SemanticActionKind::McpToolCall
                    && child.kind == SemanticActionKind::McpResponse
                    && (child_references_parent(child, attrs::mcp::TOOL_CALL_ACTION_ID, parent)
                        || child.action_id == mcp_child_action_id(&parent.action_id, "response"))
            }
            SemanticActionLinkRole::McpRequestStdout => {
                parent.kind == SemanticActionKind::McpRequest
                    && child.kind == SemanticActionKind::McpStdout
                    && (child_references_parent(child, attrs::mcp::REQUEST_ACTION_ID, parent)
                        || child.action_id
                            == mcp_sibling_action_id(&parent.action_id, "request", "stdout"))
            }
            SemanticActionLinkRole::McpRequestClientSend => {
                parent.kind == SemanticActionKind::McpRequest
                    && child.kind == SemanticActionKind::McpClientSend
                    && (child_references_parent(child, attrs::mcp::REQUEST_ACTION_ID, parent)
                        || child.action_id
                            == mcp_sibling_action_id(&parent.action_id, "request", "client_send"))
            }
            SemanticActionLinkRole::McpResponseStdin => {
                parent.kind == SemanticActionKind::McpResponse
                    && child.kind == SemanticActionKind::McpStdin
                    && (child_references_parent(child, attrs::mcp::RESPONSE_ACTION_ID, parent)
                        || child.action_id
                            == mcp_sibling_action_id(&parent.action_id, "response", "stdin"))
            }
            SemanticActionLinkRole::McpResponseClientReceive => {
                parent.kind == SemanticActionKind::McpResponse
                    && child.kind == SemanticActionKind::McpClientReceive
                    && (child_references_parent(child, attrs::mcp::RESPONSE_ACTION_ID, parent)
                        || child.action_id
                            == mcp_sibling_action_id(
                                &parent.action_id,
                                "response",
                                "client_receive",
                            ))
            }
            _ => false,
        }
}

fn child_references_parent(child: &SemanticAction, attr: &str, parent: &SemanticAction) -> bool {
    child
        .attributes
        .get(attr)
        .is_some_and(|action_id| action_id == &parent.action_id)
}

fn mcp_child_action_id(tool_call_action_id: &str, suffix: &str) -> String {
    format!("{tool_call_action_id}:{suffix}")
}

fn mcp_parent_action_id(action_id: &str, suffix: &str) -> String {
    action_id
        .strip_suffix(&format!(":{suffix}"))
        .unwrap_or(action_id)
        .to_string()
}

fn mcp_sibling_action_id(action_id: &str, from_suffix: &str, to_suffix: &str) -> String {
    format!(
        "{}:{to_suffix}",
        mcp_parent_action_id(action_id, from_suffix)
    )
}
