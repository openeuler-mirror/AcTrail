//! Private action and diagnostic construction for tool projection.

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};
use serde_json::Value;

use crate::live::tool::internal::declaration::DeclaredLlmToolCall;
use crate::llm_pipeline::{ProjectedLlmToolResult, canonical_llm_json};

pub(super) fn tool_diagnostic(
    trace_id: TraceId,
    process: &model_core::process::ProcessIdentity,
    observed_at: SystemTime,
    code: LlmPipelineDiagnosticCode,
    severity: LlmPipelineDiagnosticSeverity,
) -> LlmPipelineDiagnostic {
    LlmPipelineDiagnostic::new(
        trace_id,
        process,
        observed_at,
        code,
        severity,
        LlmPipelineDiagnosticStage::ToolProjection,
    )
}

pub(super) fn tool_call_action(
    response: &SemanticAction,
    declared: &DeclaredLlmToolCall,
) -> SemanticAction {
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

pub(super) fn agent_invocation_action(
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

pub(super) fn tool_result_action(
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
