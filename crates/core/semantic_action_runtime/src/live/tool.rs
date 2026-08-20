use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use config_core::daemon::AgentInvocationConfig;
use model_core::ids::TraceId;
use semantic_action::{
    LlmRequestLineageWrite, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionLink, SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus,
    attr_keys as attrs,
};
use serde_json::{Value, json};

use crate::payload_projection::llm::{ProjectedLlmToolResult, canonical_llm_json};

#[derive(Default)]
pub(super) struct ToolInteractionOutput {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) links: Vec<SemanticActionLink>,
}

#[derive(Clone, Debug)]
pub(super) struct DeclaredLlmToolCall {
    pub(super) ordinal: usize,
    pub(super) tool_call_id: Option<String>,
    pub(super) name: String,
    arguments: Value,
    arguments_json: String,
    arguments_hash: String,
}

impl DeclaredLlmToolCall {
    pub(super) fn from_response(action: &SemanticAction) -> Vec<Self> {
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
                let name = tool_call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .or_else(|| tool_call.get("name").and_then(Value::as_str))?
                    .to_string();
                let arguments = tool_call
                    .pointer("/function/arguments")
                    .or_else(|| tool_call.get("arguments"))
                    .or_else(|| tool_call.get("input"))
                    .map(parse_arguments)
                    .unwrap_or(Value::Null);
                let (arguments_json, arguments_hash) = canonical_llm_json(&arguments);
                Some(Self {
                    ordinal,
                    tool_call_id: tool_call
                        .get("id")
                        .or_else(|| tool_call.get("call_id"))
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(ToString::to_string),
                    name,
                    arguments,
                    arguments_json,
                    arguments_hash,
                })
            })
            .collect()
    }
}

fn parse_arguments(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| value.clone())
}

#[derive(Clone)]
struct ToolCallRecord {
    action: SemanticAction,
    tool_call_id: Option<String>,
}

struct AgentInvocationRecord {
    action: SemanticAction,
    prompt_message_hashes: BTreeSet<String>,
    prompt_preview: Option<String>,
    child_linked: bool,
}

pub(super) struct ToolInteractionProjector {
    agent_invocation_enabled: bool,
    agent_tool_names: BTreeSet<String>,
    tool_calls: BTreeMap<(TraceId, String), ToolCallRecord>,
    tool_results: BTreeMap<(TraceId, String), SemanticAction>,
    agent_invocations: BTreeMap<(TraceId, String), AgentInvocationRecord>,
}

impl ToolInteractionProjector {
    pub(super) fn new(config: AgentInvocationConfig) -> Self {
        Self {
            agent_invocation_enabled: config.enabled,
            agent_tool_names: config.tool_names.into_iter().collect(),
            tool_calls: BTreeMap::new(),
            tool_results: BTreeMap::new(),
            agent_invocations: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn observe_actions(&mut self, actions: &[SemanticAction]) -> ToolInteractionOutput {
        self.observe_llm_output(actions, &[], &[])
    }

    pub(super) fn observe_llm_output(
        &mut self,
        actions: &[SemanticAction],
        tool_results: &[ProjectedLlmToolResult],
        lineages: &[LlmRequestLineageWrite],
    ) -> ToolInteractionOutput {
        let mut output = ToolInteractionOutput::default();
        output.links.extend(lineage_links(lineages));

        for response in actions
            .iter()
            .filter(|action| action.kind == SemanticActionKind::LlmResponse)
        {
            self.observe_response(response, &mut output);
        }

        let requests = actions
            .iter()
            .filter(|action| action.kind == SemanticActionKind::LlmRequest)
            .map(|action| (action.action_id.as_str(), action))
            .collect::<BTreeMap<_, _>>();
        for result in tool_results {
            let Some(request) = requests.get(result.request_action_id.as_str()) else {
                continue;
            };
            self.observe_result(request, result, &mut output);
        }
        for request in requests.values() {
            let continuing_trajectory = lineages.iter().any(|lineage| {
                lineage.action_id == request.action_id && lineage.parent_action_id.is_some()
            });
            self.link_agent_child(request, continuing_trajectory, &mut output.links);
        }
        output
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> ToolInteractionOutput {
        let mut finalized = ToolInteractionOutput::default();
        for ((candidate, tool_call_action_id), invocation) in &mut self.agent_invocations {
            if *candidate == trace_id
                && invocation.action.status == SemanticActionStatus::InProgress
            {
                invocation.action.status = SemanticActionStatus::Unknown;
                invocation.action.completeness = SemanticActionCompleteness::Partial;
                invocation.action.end_time = Some(finished_at);
                invocation.action.attributes.insert(
                    attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                    "true".to_string(),
                );
                finalized.actions.push(invocation.action.clone());
                if let Some(tool_call) = self
                    .tool_calls
                    .get(&(*candidate, tool_call_action_id.clone()))
                {
                    finalized.links.push(action_link(
                        &tool_call.action,
                        &invocation.action,
                        SemanticActionLinkRole::LlmToolCallAgentInvocation,
                        SemanticActionLinkConfidence::Observed,
                    ));
                }
            }
        }
        self.forget_trace(trace_id);
        finalized
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.tool_calls
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.tool_results
            .retain(|(candidate, _), _| *candidate != trace_id);
        self.agent_invocations
            .retain(|(candidate, _), _| *candidate != trace_id);
    }

    fn observe_response(&mut self, response: &SemanticAction, output: &mut ToolInteractionOutput) {
        for declared in DeclaredLlmToolCall::from_response(response) {
            let action = tool_call_action(response, &declared);
            let key = (action.trace_id, action.action_id.clone());
            let changed = self
                .tool_calls
                .get(&key)
                .is_none_or(|record| record.action != action);
            self.tool_calls.insert(
                key.clone(),
                ToolCallRecord {
                    action: action.clone(),
                    tool_call_id: declared.tool_call_id.clone(),
                },
            );
            if changed {
                output.actions.push(action.clone());
                output.links.push(action_link(
                    response,
                    &action,
                    SemanticActionLinkRole::LlmResponseToolCall,
                    SemanticActionLinkConfidence::Observed,
                ));
            }
            if self.agent_invocation_enabled && self.agent_tool_names.contains(&declared.name) {
                self.observe_agent_invocation(&action, &declared, output);
            }
        }
    }

    fn observe_agent_invocation(
        &mut self,
        tool_call: &SemanticAction,
        declared: &DeclaredLlmToolCall,
        output: &mut ToolInteractionOutput,
    ) {
        let key = (tool_call.trace_id, tool_call.action_id.clone());
        if self.agent_invocations.contains_key(&key) {
            return;
        }
        let invocation = agent_invocation_action(tool_call, declared);
        let (prompt_message_hashes, prompt_preview) = declared
            .arguments
            .get("prompt")
            .and_then(Value::as_str)
            .map(|prompt| {
                (
                    prompt_message_hashes(prompt),
                    Some(tool_prompt_preview(prompt)),
                )
            })
            .unwrap_or_default();
        output.actions.push(invocation.clone());
        output.links.push(action_link(
            tool_call,
            &invocation,
            SemanticActionLinkRole::LlmToolCallAgentInvocation,
            SemanticActionLinkConfidence::Observed,
        ));
        self.agent_invocations.insert(
            key,
            AgentInvocationRecord {
                action: invocation,
                prompt_message_hashes,
                prompt_preview,
                child_linked: false,
            },
        );
    }

    fn observe_result(
        &mut self,
        request: &SemanticAction,
        result: &ProjectedLlmToolResult,
        output: &mut ToolInteractionOutput,
    ) {
        let matching_calls = result.tool_call_id.as_deref().map_or_else(Vec::new, |id| {
            self.tool_calls
                .iter()
                .filter(|((trace_id, _), record)| {
                    *trace_id == request.trace_id
                        && record.action.process == request.process
                        && record.tool_call_id.as_deref() == Some(id)
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>()
        });
        let binding_state = match (result.tool_call_id.as_deref(), matching_calls.len()) {
            (None, _) => "missing_id",
            (Some(_), 0) => "unmatched",
            (Some(_), 1) => "bound",
            (Some(_), _) => "ambiguous",
        };
        let action = tool_result_action(request, result, binding_state);
        let result_key = (action.trace_id, action.action_id.clone());
        let changed = self.tool_results.get(&result_key) != Some(&action);
        self.tool_results.insert(result_key, action.clone());
        if changed {
            output.actions.push(action.clone());
        }
        let [call_key] = matching_calls.as_slice() else {
            return;
        };
        let Some(call) = self.tool_calls.get(call_key) else {
            return;
        };
        let call_action = call.action.clone();
        if changed {
            output.links.push(action_link(
                &call_action,
                &action,
                SemanticActionLinkRole::LlmToolCallResult,
                SemanticActionLinkConfidence::Observed,
            ));
        }
        if let Some(invocation) = self.agent_invocations.get_mut(call_key)
            && invocation.action.status == SemanticActionStatus::InProgress
        {
            invocation.action.status = if result.is_error {
                SemanticActionStatus::Error
            } else {
                SemanticActionStatus::Success
            };
            invocation.action.completeness = request.completeness;
            invocation.action.end_time = request.end_time.or(Some(request.start_time));
            output.actions.push(invocation.action.clone());
            output.links.push(action_link(
                &call_action,
                &invocation.action,
                SemanticActionLinkRole::LlmToolCallAgentInvocation,
                SemanticActionLinkConfidence::Observed,
            ));
        }
    }

    fn link_agent_child(
        &mut self,
        request: &SemanticAction,
        continuing_trajectory: bool,
        output: &mut Vec<SemanticActionLink>,
    ) {
        if continuing_trajectory
            || request
                .attributes
                .contains_key(attrs::llm_request::BACKGROUND_KIND)
        {
            return;
        }
        let request_hash = request
            .attributes
            .get(attrs::llm_request::LATEST_USER_MESSAGE_HASH);
        let request_preview = request.attributes.get(attrs::llm_request::MESSAGE_PREVIEW);
        let matches = self
            .agent_invocations
            .iter()
            .filter(|((trace_id, _), invocation)| {
                *trace_id == request.trace_id
                    && !invocation.child_linked
                    && (request_hash
                        .is_some_and(|hash| invocation.prompt_message_hashes.contains(hash))
                        || request_preview.is_some_and(|preview| {
                            invocation.prompt_preview.as_ref() == Some(preview)
                        }))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let [key] = matches.as_slice() else {
            return;
        };
        let Some(invocation) = self.agent_invocations.get_mut(key) else {
            return;
        };
        invocation.child_linked = true;
        output.push(action_link(
            &invocation.action,
            request,
            SemanticActionLinkRole::AgentInvocationChildLlmRequest,
            SemanticActionLinkConfidence::Derived,
        ));
    }
}

fn tool_call_action(response: &SemanticAction, declared: &DeclaredLlmToolCall) -> SemanticAction {
    let mut attributes = BTreeMap::from([
        (
            attrs::llm_tool_call::NAME.to_string(),
            declared.name.clone(),
        ),
        (
            attrs::llm_tool_call::ORDINAL.to_string(),
            declared.ordinal.to_string(),
        ),
        (
            attrs::llm_tool_call::RESPONSE_ACTION_ID.to_string(),
            response.action_id.clone(),
        ),
        (
            attrs::llm_tool_call::ARGUMENTS_BYTES.to_string(),
            declared.arguments_json.len().to_string(),
        ),
        (
            attrs::llm_tool_call::ARGUMENTS_HASH.to_string(),
            declared.arguments_hash.clone(),
        ),
    ]);
    if let Some(id) = &declared.tool_call_id {
        attributes.insert(attrs::llm_tool_call::ID.to_string(), id.clone());
    }
    SemanticAction {
        action_id: format!("{}:tool-call:{}", response.action_id, declared.ordinal),
        trace_id: response.trace_id,
        kind: SemanticActionKind::LlmToolCall,
        title: format!("LLM tool call {}", declared.name),
        start_time: response.start_time,
        end_time: response.end_time,
        process: response.process,
        status: response.status,
        completeness: response.completeness,
        attributes,
        evidence: response.evidence.clone(),
    }
}

fn agent_invocation_action(
    tool_call: &SemanticAction,
    declared: &DeclaredLlmToolCall,
) -> SemanticAction {
    let agent_type = declared
        .arguments
        .get("subagent_type")
        .or_else(|| declared.arguments.get("agent_type"))
        .and_then(Value::as_str);
    let mut attributes = BTreeMap::from([
        (
            attrs::agent_invocation::EVIDENCE_ACTION_ID.to_string(),
            tool_call.action_id.clone(),
        ),
        (
            attrs::agent_invocation::TOOL_CALL_ACTION_ID.to_string(),
            tool_call.action_id.clone(),
        ),
        (
            attrs::agent_invocation::TOOL_NAME.to_string(),
            declared.name.clone(),
        ),
        (
            attrs::agent_invocation::TRIGGER.to_string(),
            "llm_tool_call".to_string(),
        ),
    ]);
    if let Some(id) = &declared.tool_call_id {
        attributes.insert(
            attrs::agent_invocation::TOOL_CALL_ID.to_string(),
            id.clone(),
        );
    }
    if let Some(agent_type) = agent_type {
        attributes.insert(
            attrs::agent_invocation::AGENT_TYPE.to_string(),
            agent_type.to_string(),
        );
    }
    if let Some(prompt) = declared.arguments.get("prompt").and_then(Value::as_str) {
        let (_, hash) = canonical_llm_json(&Value::String(prompt.to_string()));
        attributes.insert(attrs::agent_invocation::PROMPT_HASH.to_string(), hash);
    }
    SemanticAction {
        action_id: format!("{}:agent-invocation", tool_call.action_id),
        trace_id: tool_call.trace_id,
        kind: SemanticActionKind::AgentInvocation,
        title: agent_type.map_or_else(
            || format!("Invoke agent via {}", declared.name),
            |kind| format!("Invoke {kind} agent"),
        ),
        start_time: tool_call.start_time,
        end_time: None,
        process: tool_call.process,
        status: SemanticActionStatus::InProgress,
        completeness: SemanticActionCompleteness::Partial,
        attributes,
        evidence: tool_call.evidence.clone(),
    }
}

fn tool_result_action(
    request: &SemanticAction,
    result: &ProjectedLlmToolResult,
    binding_state: &str,
) -> SemanticAction {
    let mut attributes = BTreeMap::from([
        (
            attrs::llm_tool_result::BINDING_STATE.to_string(),
            binding_state.to_string(),
        ),
        (
            attrs::llm_tool_result::CONTENT_BYTES.to_string(),
            result.content_bytes.to_string(),
        ),
        (
            attrs::llm_tool_result::CONTENT_HASH.to_string(),
            result.content_hash.clone(),
        ),
        (
            attrs::llm_tool_result::CONTENT_EXPORT_STATE.to_string(),
            result.content_export_state.to_string(),
        ),
        (
            attrs::llm_tool_result::IS_ERROR.to_string(),
            result.is_error.to_string(),
        ),
        (
            attrs::llm_tool_result::ORDINAL.to_string(),
            result.ordinal.to_string(),
        ),
        (
            attrs::llm_tool_result::REQUEST_ACTION_ID.to_string(),
            request.action_id.clone(),
        ),
    ]);
    if let Some(id) = &result.tool_call_id {
        attributes.insert(attrs::llm_tool_result::ID.to_string(), id.clone());
    }
    if let Some(content_json) = &result.content_json {
        attributes.insert(
            attrs::llm_tool_result::CONTENT_JSON.to_string(),
            content_json.clone(),
        );
    }
    SemanticAction {
        action_id: format!("{}:tool-result:{}", request.action_id, result.ordinal),
        trace_id: request.trace_id,
        kind: SemanticActionKind::LlmToolResult,
        title: "LLM tool result".to_string(),
        start_time: request.start_time,
        end_time: request.end_time,
        process: request.process,
        status: if result.is_error {
            SemanticActionStatus::Error
        } else {
            SemanticActionStatus::Success
        },
        completeness: request.completeness,
        attributes,
        evidence: request.evidence.clone(),
    }
}

fn lineage_links(lineages: &[LlmRequestLineageWrite]) -> Vec<SemanticActionLink> {
    lineages
        .iter()
        .flat_map(|lineage| {
            let parent = lineage.parent_action_id.as_ref().map(|parent| {
                lineage_link(
                    lineage,
                    parent,
                    SemanticActionLinkRole::LlmRequestTrajectoryParent,
                )
            });
            let fork = lineage.forked_from_action_id.as_ref().map(|parent| {
                lineage_link(
                    lineage,
                    parent,
                    SemanticActionLinkRole::LlmRequestTrajectoryFork,
                )
            });
            parent.into_iter().chain(fork)
        })
        .collect()
}

fn lineage_link(
    lineage: &LlmRequestLineageWrite,
    parent_action_id: &str,
    role: SemanticActionLinkRole,
) -> SemanticActionLink {
    SemanticActionLink {
        trace_id: lineage.trace_id,
        parent_action_id: parent_action_id.to_string(),
        child_action_id: lineage.action_id.clone(),
        role,
        confidence: SemanticActionLinkConfidence::Derived,
        valid: true,
        evidence: Vec::new(),
        attributes: BTreeMap::new(),
    }
}

fn action_link(
    parent: &SemanticAction,
    child: &SemanticAction,
    role: SemanticActionLinkRole,
    confidence: SemanticActionLinkConfidence,
) -> SemanticActionLink {
    SemanticActionLink {
        trace_id: child.trace_id,
        parent_action_id: parent.action_id.clone(),
        child_action_id: child.action_id.clone(),
        role,
        confidence,
        valid: true,
        evidence: child.evidence.clone(),
        attributes: BTreeMap::new(),
    }
}

fn prompt_message_hashes(prompt: &str) -> BTreeSet<String> {
    [
        json!({"role": "user", "content": prompt}),
        json!({"role": "human", "content": prompt}),
        json!({"role": "user", "content": [{"type": "text", "text": prompt}]}),
        json!({"role": "user", "content": [{"type": "input_text", "text": prompt}]}),
        json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": prompt}]}),
    ]
    .into_iter()
    .map(|message| canonical_llm_json(&message).1)
    .collect()
}

fn tool_prompt_preview(prompt: &str) -> String {
    let mut preview = String::new();
    for (index, ch) in prompt.trim().chars().enumerate() {
        if index >= 160 {
            preview.push_str("...");
            break;
        }
        preview.push(ch);
    }
    preview
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    use config_core::daemon::AgentInvocationConfig;
    use model_core::ids::TraceId;
    use model_core::process::ProcessIdentity;
    use semantic_action::{
        LlmRequestLineageWrite, LlmTrajectoryStartReason, LlmTrajectoryTransition, SemanticAction,
        SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkRole,
        SemanticActionStatus, attr_keys as attrs,
    };

    use crate::payload_projection::llm::ProjectedLlmToolResult;

    use super::{ToolInteractionProjector, prompt_message_hashes};

    #[test]
    fn declared_agent_tool_becomes_tool_call_and_agent_invocation() {
        let response = llm_response(
            r#"[{"id":"call_agent_1","type":"function","function":{"name":"Agent","arguments":"{\"subagent_type\":\"general-purpose\"}"}}]"#,
        );
        let mut projector = ToolInteractionProjector::new(AgentInvocationConfig::default());

        let output = projector.observe_actions(&[response]);

        assert_eq!(
            output
                .actions
                .iter()
                .filter(|action| action.kind == SemanticActionKind::LlmToolCall)
                .count(),
            1
        );
        assert_eq!(
            output
                .actions
                .iter()
                .filter(|action| action.kind == SemanticActionKind::AgentInvocation)
                .count(),
            1
        );
        assert!(
            output
                .links
                .iter()
                .any(|link| link.role == SemanticActionLinkRole::LlmResponseToolCall)
        );
        assert!(
            output
                .links
                .iter()
                .any(|link| { link.role == SemanticActionLinkRole::LlmToolCallAgentInvocation })
        );
    }

    #[test]
    fn explicit_tool_result_id_links_and_completes_agent_invocation() {
        let response =
            llm_response(r#"[{"id":"call_agent_1","function":{"name":"Agent","arguments":"{}"}}]"#);
        let request = llm_request("llm-request-2", BTreeMap::new());
        let result = ProjectedLlmToolResult {
            request_action_id: request.action_id.clone(),
            tool_call_id: Some("call_agent_1".to_string()),
            ordinal: 0,
            is_error: false,
            content_json: None,
            content_hash: "sha256:result".to_string(),
            content_bytes: 12,
            content_export_state: "none",
        };
        let mut projector = ToolInteractionProjector::new(AgentInvocationConfig::default());
        projector.observe_actions(&[response]);

        let output = projector.observe_llm_output(&[request], &[result], &[]);

        assert!(
            output
                .actions
                .iter()
                .any(|action| action.kind == SemanticActionKind::LlmToolResult)
        );
        assert!(
            output
                .links
                .iter()
                .any(|link| { link.role == SemanticActionLinkRole::LlmToolCallResult })
        );
        assert!(
            output
                .links
                .iter()
                .any(|link| { link.role == SemanticActionLinkRole::LlmToolCallAgentInvocation })
        );
        assert!(output.actions.iter().any(|action| {
            action.kind == SemanticActionKind::AgentInvocation
                && action.status == SemanticActionStatus::Success
        }));
    }

    #[test]
    fn unique_prompt_hash_links_agent_invocation_to_child_request() {
        let prompt = "Inspect the delegated fixture";
        let response = llm_response(&format!(
            r#"[{{"id":"call_agent_1","function":{{"name":"Agent","arguments":{{"prompt":{}}}}}}}]"#,
            serde_json::to_string(prompt).unwrap()
        ));
        let hash = prompt_message_hashes(prompt).into_iter().next().unwrap();
        let request = llm_request(
            "child-request",
            BTreeMap::from([(
                attrs::llm_request::LATEST_USER_MESSAGE_HASH.to_string(),
                hash,
            )]),
        );
        let mut projector = ToolInteractionProjector::new(AgentInvocationConfig::default());
        projector.observe_actions(&[response]);

        let output = projector.observe_actions(&[request]);

        assert!(
            output.links.iter().any(|link| {
                link.role == SemanticActionLinkRole::AgentInvocationChildLlmRequest
            })
        );
    }

    #[test]
    fn duplicate_prompt_candidates_do_not_guess_a_subagent_parent() {
        let prompt = "same delegated task";
        let responses = ["response-1", "response-2"].map(|action_id| {
            let mut response = llm_response(&format!(
                r#"[{{"id":"{action_id}","function":{{"name":"Agent","arguments":{{"prompt":{}}}}}}}]"#,
                serde_json::to_string(prompt).unwrap()
            ));
            response.action_id = action_id.to_string();
            response
        });
        let request = llm_request(
            "child-request",
            BTreeMap::from([(
                attrs::llm_request::MESSAGE_PREVIEW.to_string(),
                prompt.to_string(),
            )]),
        );
        let mut projector = ToolInteractionProjector::new(AgentInvocationConfig::default());
        projector.observe_actions(&responses);

        let output = projector.observe_actions(&[request]);

        assert!(
            !output.links.iter().any(|link| {
                link.role == SemanticActionLinkRole::AgentInvocationChildLlmRequest
            })
        );
    }

    #[test]
    fn continuing_trajectory_is_not_guessed_as_a_subagent_child() {
        let prompt = "continue the delegated-looking task";
        let response = llm_response(&format!(
            r#"[{{"id":"call_agent_1","function":{{"name":"Agent","arguments":{{"prompt":{}}}}}}}]"#,
            serde_json::to_string(prompt).unwrap()
        ));
        let hash = prompt_message_hashes(prompt).into_iter().next().unwrap();
        let request = llm_request(
            "request-2",
            BTreeMap::from([(
                attrs::llm_request::LATEST_USER_MESSAGE_HASH.to_string(),
                hash,
            )]),
        );
        let lineage = LlmRequestLineageWrite {
            trace_id: request.trace_id,
            action_id: request.action_id.clone(),
            trajectory_id: "trajectory-1".to_string(),
            parent_action_id: Some("request-1".to_string()),
            forked_from_action_id: None,
            trajectory_position: 1,
            transition: LlmTrajectoryTransition::Append,
            start_reason: LlmTrajectoryStartReason::Unspecified,
            inference_version: 2,
        };
        let mut projector = ToolInteractionProjector::new(AgentInvocationConfig::default());
        projector.observe_actions(&[response]);

        let output = projector.observe_llm_output(&[request], &[], &[lineage]);

        assert!(
            !output.links.iter().any(|link| {
                link.role == SemanticActionLinkRole::AgentInvocationChildLlmRequest
            })
        );
        assert!(
            output
                .links
                .iter()
                .any(|link| { link.role == SemanticActionLinkRole::LlmRequestTrajectoryParent })
        );
    }

    #[test]
    fn request_lineage_rows_become_typed_graph_edges() {
        let lineage = LlmRequestLineageWrite {
            trace_id: TraceId::new(1),
            action_id: "request-2".to_string(),
            trajectory_id: "trajectory-1".to_string(),
            parent_action_id: Some("request-1".to_string()),
            forked_from_action_id: None,
            trajectory_position: 1,
            transition: LlmTrajectoryTransition::Append,
            start_reason: LlmTrajectoryStartReason::Unspecified,
            inference_version: 2,
        };
        let mut projector = ToolInteractionProjector::new(AgentInvocationConfig::default());

        let output = projector.observe_llm_output(&[], &[], &[lineage]);

        assert!(output.links.iter().any(|link| {
            link.parent_action_id == "request-1"
                && link.child_action_id == "request-2"
                && link.role == SemanticActionLinkRole::LlmRequestTrajectoryParent
        }));
    }

    fn llm_response(tool_calls_json: &str) -> SemanticAction {
        semantic_action(
            "llm-response-1",
            SemanticActionKind::LlmResponse,
            BTreeMap::from([(
                attrs::llm_response::TOOL_CALLS_JSON.to_string(),
                tool_calls_json.to_string(),
            )]),
        )
    }

    fn llm_request(action_id: &str, attributes: BTreeMap<String, String>) -> SemanticAction {
        semantic_action(action_id, SemanticActionKind::LlmRequest, attributes)
    }

    fn semantic_action(
        action_id: &str,
        kind: SemanticActionKind,
        attributes: BTreeMap<String, String>,
    ) -> SemanticAction {
        SemanticAction {
            action_id: action_id.to_string(),
            trace_id: TraceId::new(1),
            kind,
            title: kind.as_str().to_string(),
            start_time: SystemTime::UNIX_EPOCH,
            end_time: Some(SystemTime::UNIX_EPOCH),
            process: ProcessIdentity::new(7),
            status: SemanticActionStatus::Success,
            completeness: SemanticActionCompleteness::Complete,
            attributes,
            evidence: Vec::new(),
        }
    }
}
