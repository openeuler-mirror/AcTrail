use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionKind, attr_keys as attrs};
use serde_json::Value;

use super::projector::LiveMcpProjector;

#[derive(Default)]
pub(super) struct McpAttributionState {
    llm_tool_proposals: BTreeMap<TraceId, Vec<LlmToolCallProposal>>,
    consumed_llm_tool_proposals: BTreeMap<(TraceId, String), BTreeSet<usize>>,
    unattributed_mcp_tool_calls: BTreeMap<TraceId, Vec<SemanticAction>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LlmToolCallProposal {
    ordinal: usize,
    response_action_id: String,
    response_start_time: SystemTime,
    tool_call_id: Option<String>,
    name: String,
}

impl LiveMcpProjector {
    pub(in crate::live) fn observe_llm_actions(
        &mut self,
        actions: &[SemanticAction],
    ) -> Vec<SemanticAction> {
        if !self.enabled {
            return Vec::new();
        }
        let mut traces_with_proposals = BTreeSet::new();
        for action in actions {
            if action.kind != SemanticActionKind::LlmResponse {
                continue;
            }
            let consumed = self
                .attribution
                .consumed_llm_tool_proposals
                .get(&(action.trace_id, action.action_id.clone()))
                .cloned()
                .unwrap_or_default();
            let proposals = LlmToolCallProposal::from_response(action)
                .into_iter()
                .filter(|proposal| !consumed.contains(&proposal.ordinal))
                .collect::<Vec<_>>();
            if proposals.is_empty() {
                continue;
            }
            let trace_proposals = self
                .attribution
                .llm_tool_proposals
                .entry(action.trace_id)
                .or_default();
            trace_proposals.retain(|proposal| proposal.response_action_id != action.action_id);
            trace_proposals.extend(proposals);
            traces_with_proposals.insert(action.trace_id);
        }
        traces_with_proposals
            .into_iter()
            .flat_map(|trace_id| self.reconcile_unattributed_mcp_tool_calls(trace_id))
            .collect()
    }

    pub(super) fn apply_llm_proposal_attributes(
        &mut self,
        trace_id: TraceId,
        server_name: &str,
        tool_name: &str,
        action_start_time: SystemTime,
        attributes: &mut BTreeMap<String, String>,
    ) -> bool {
        let Some(proposal) = self.take_matching_llm_tool_proposal(
            trace_id,
            server_name,
            tool_name,
            action_start_time,
        ) else {
            return false;
        };
        attributes.insert(
            attrs::llm_response::ACTION_ID.to_string(),
            proposal.response_action_id,
        );
        if let Some(tool_call_id) = proposal.tool_call_id {
            attributes.insert(attrs::llm_tool_call::ID.to_string(), tool_call_id);
        }
        attributes.insert(attrs::llm_tool_call::NAME.to_string(), proposal.name);
        true
    }

    pub(super) fn track_unattributed_mcp_tool_call(&mut self, action: &SemanticAction) {
        if action.kind != SemanticActionKind::McpToolCall {
            return;
        }
        if action
            .attributes
            .contains_key(attrs::llm_response::ACTION_ID)
        {
            let remove_trace_entry = self
                .attribution
                .unattributed_mcp_tool_calls
                .get_mut(&action.trace_id)
                .is_some_and(|pending| {
                    pending.retain(|candidate| candidate.action_id != action.action_id);
                    pending.is_empty()
                });
            if remove_trace_entry {
                self.attribution
                    .unattributed_mcp_tool_calls
                    .remove(&action.trace_id);
            }
            return;
        }
        let pending = self
            .attribution
            .unattributed_mcp_tool_calls
            .entry(action.trace_id)
            .or_default();
        if let Some(existing) = pending
            .iter_mut()
            .find(|candidate| candidate.action_id == action.action_id)
        {
            existing.clone_from(action);
        } else {
            pending.push(action.clone());
        }
    }

    pub(super) fn forget_attribution_trace(&mut self, trace_id: TraceId) {
        self.attribution
            .llm_tool_proposals
            .retain(|candidate, _| *candidate != trace_id);
        self.attribution
            .consumed_llm_tool_proposals
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.attribution
            .unattributed_mcp_tool_calls
            .retain(|candidate, _| *candidate != trace_id);
    }

    fn take_matching_llm_tool_proposal(
        &mut self,
        trace_id: TraceId,
        server_name: &str,
        tool_name: &str,
        action_start_time: SystemTime,
    ) -> Option<LlmToolCallProposal> {
        let proposal = {
            let proposals = self.attribution.llm_tool_proposals.get_mut(&trace_id)?;
            let latest_response_action_id = proposals
                .iter()
                .rev()
                .find(|proposal| proposal.matches(server_name, tool_name, action_start_time))
                .map(|proposal| proposal.response_action_id.clone())?;
            let proposal_index = proposals.iter().position(|proposal| {
                proposal.response_action_id == latest_response_action_id
                    && proposal.matches(server_name, tool_name, action_start_time)
            })?;
            proposals.remove(proposal_index)
        };
        self.attribution
            .consumed_llm_tool_proposals
            .entry((trace_id, proposal.response_action_id.clone()))
            .or_default()
            .insert(proposal.ordinal);
        Some(proposal)
    }

    fn reconcile_unattributed_mcp_tool_calls(&mut self, trace_id: TraceId) -> Vec<SemanticAction> {
        let Some(pending) = self
            .attribution
            .unattributed_mcp_tool_calls
            .remove(&trace_id)
        else {
            return Vec::new();
        };
        let mut remaining = Vec::new();
        let mut updated = Vec::new();
        for mut action in pending {
            let Some(server_name) = action.attributes.get(attrs::mcp::SERVER_NAME).cloned() else {
                remaining.push(action);
                continue;
            };
            let Some(tool_name) = action.attributes.get(attrs::mcp::TOOL_NAME).cloned() else {
                remaining.push(action);
                continue;
            };
            if !self.apply_llm_proposal_attributes(
                trace_id,
                &server_name,
                &tool_name,
                action.start_time,
                &mut action.attributes,
            ) {
                remaining.push(action);
                continue;
            }
            self.apply_attribution_to_open_call(&action);
            updated.push(action);
        }
        if !remaining.is_empty() {
            self.attribution
                .unattributed_mcp_tool_calls
                .insert(trace_id, remaining);
        }
        updated
    }

    fn apply_attribution_to_open_call(&mut self, action: &SemanticAction) {
        let Some(open) = self
            .correlation
            .open_calls
            .values_mut()
            .find(|open| open.action.action_id == action.action_id)
        else {
            return;
        };
        for key in [
            attrs::llm_response::ACTION_ID,
            attrs::llm_tool_call::ID,
            attrs::llm_tool_call::NAME,
        ] {
            if let Some(value) = action.attributes.get(key) {
                open.action
                    .attributes
                    .insert(key.to_string(), value.clone());
            }
        }
    }
}

impl LlmToolCallProposal {
    fn from_response(action: &SemanticAction) -> Vec<Self> {
        let Some(tool_calls_json) = action.attributes.get(attrs::llm_response::TOOL_CALLS_JSON)
        else {
            return Vec::new();
        };
        let Ok(Value::Array(tool_calls)) = serde_json::from_str::<Value>(tool_calls_json) else {
            return Vec::new();
        };
        tool_calls
            .into_iter()
            .enumerate()
            .filter_map(|(ordinal, tool_call)| {
                // MCP attribution needs identity only. Argument canonicalization and hashing
                // belong to the tool projector and must not be repeated on this hot path.
                let name = tool_call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .or_else(|| tool_call.get("name").and_then(Value::as_str))?;
                let encoded_name = name.strip_prefix("mcp__")?;
                if !encoded_name.contains("__") {
                    return None;
                }
                Some(Self {
                    ordinal,
                    response_action_id: action.action_id.clone(),
                    response_start_time: action.start_time,
                    tool_call_id: tool_call
                        .get("id")
                        .or_else(|| tool_call.get("call_id"))
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(ToString::to_string),
                    name: name.to_string(),
                })
            })
            .collect()
    }

    fn matches(
        &self,
        server_name: &str,
        tool_name: &str,
        mcp_action_start_time: SystemTime,
    ) -> bool {
        if self.response_start_time > mcp_action_start_time {
            return false;
        }
        let Some(encoded_tool_name) = self
            .name
            .strip_prefix("mcp__")
            .and_then(|name| name.strip_prefix(server_name))
            .and_then(|name| name.strip_prefix("__"))
            .filter(|name| !name.is_empty())
        else {
            return false;
        };
        encoded_tool_name == tool_name
            || tool_name
                .strip_prefix(server_name)
                .and_then(|name| name.strip_prefix('.'))
                == Some(encoded_tool_name)
    }
}
