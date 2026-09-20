//! Semantic action tree JSON for the web UI.

use std::path::Path;
use std::time::SystemTime;

use model_core::ids::TraceId;
use semantic_action::{
    FilePathSetPath, FilePathSetPathPage, LlmRequestContentPage, LlmRequestLineage,
    McpJsonRpcContentPage, SemanticAction,
};
use serde_json::{Value, json as json_value};
use storage_core::{
    SemanticActionChildPageQuery, SemanticActionSummary, StorageBackend, StorageError,
};

use super::action_tree_projection::{
    ActionDisplayProjection, DisplayChild, LlmCallDisplay, LlmToolDisplay,
};
use super::action_tree_roles::{DISPLAY_PARENT_ROLES, NODE_ID_AGENT, ROOT_LINK_ROLES};
use super::agent_idle::{AgentIdleInterval, AgentIdleProjection};
use super::projection_cache;
use super::{LlmNavMode, LlmRequestContentNodeQuery};
use crate::json;

#[path = "actions/serialization.rs"]
mod serialization;

pub(super) use serialization::{action_json, action_json_lite};
use serialization::{link_json, link_json_lite};

const WATERFALL_INITIAL_ACTION_KINDS: &[&str] = &[
    "command.invocation",
    "agent.invocation",
    "llm.call",
    "llm.request",
    "llm.response",
];
const WATERFALL_INITIAL_LINK_ROLES: &[&str] = &[
    "agent.performed_action",
    "agent.invocation.exec",
    "agent.invocation.child_llm_request",
    "command.contains_command_invocation",
    "command.contains_llm_call",
    "llm.call.request",
    "llm.call.response",
    "llm.request.llm_response",
];

pub(super) fn action_tree_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    projection_cache::sync_trace_revision(storage, storage_path, trace_id);
    let projection =
        projection_cache::cached_action_display_projection(storage_path, trace_id, || {
            ActionDisplayProjection::load(storage, trace_id)
        })?;
    let roots = projection
        .root_action_ids
        .iter()
        .map(|action_id| json::string(action_id))
        .collect::<Vec<_>>();
    let actions = projection
        .actions
        .iter()
        .map(action_json_lite)
        .collect::<Vec<_>>();
    let links = projection
        .links
        .iter()
        .map(link_json_lite)
        .collect::<Vec<_>>();
    let associations = projection
        .associations
        .iter()
        .map(link_json_lite)
        .collect::<Vec<_>>();
    let idle_intervals = AgentIdleProjection::load(storage, trace_id)?;
    let idle_intervals_json = idle_intervals
        .iter()
        .map(AgentIdleInterval::json)
        .collect::<Vec<_>>();
    let axis_end = axis_end_unix_nanos(&projection.actions, &idle_intervals);
    Ok(format!(
        "{{\"roots\":[{}],\"actions\":[{}],\"links\":[{}],\"associations\":[{}],\"idle_intervals\":[{}],\"axis_end_unix_nanos\":{}}}",
        roots.join(","),
        actions.join(","),
        links.join(","),
        associations.join(","),
        idle_intervals_json.join(","),
        axis_end,
    ))
}

pub(super) fn waterfall_initial_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let mut actions = storage
        .semantic_actions_matching_kinds_lite(trace_id, WATERFALL_INITIAL_ACTION_KINDS)
        .map_err(|error| storage_error("read waterfall actions", error))?;
    LlmCallDisplay::normalize_loaded(&mut actions);
    let links = storage
        .semantic_action_links_matching_roles(trace_id, WATERFALL_INITIAL_LINK_ROLES)
        .map_err(|error| storage_error("read waterfall links", error))?;
    let idle_intervals = AgentIdleProjection::load(storage, trace_id)?;
    let selected = actions.len();
    let axis_end = axis_end_unix_nanos(&actions, &idle_intervals);
    let actions = actions.iter().map(action_json_lite).collect::<Vec<_>>();
    let links = links.iter().map(link_json_lite).collect::<Vec<_>>();
    let idle_intervals = idle_intervals
        .iter()
        .map(AgentIdleInterval::json)
        .collect::<Vec<_>>();
    Ok(format!(
        "{{\"actions\":[{}],\"links\":[{}],\"idle_intervals\":[{}],\"axis_end_unix_nanos\":{},\"selected_actions\":{},\"partial\":true}}",
        actions.join(","),
        links.join(","),
        idle_intervals.join(","),
        axis_end,
        json::number(selected)
    ))
}

pub(super) fn action_tree_root_json(
    _storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let summary = storage
        .semantic_action_summary(trace_id)
        .map_err(|error| storage_error("read semantic action summary", error))?;
    let observed_agent = storage
        .observed_agent_semantic_action(trace_id)
        .map_err(|error| storage_error("read observed agent action", error))?;
    let display_roles = display_parent_role_names();
    let root_child_count = storage
        .semantic_action_display_root_child_count(trace_id, &display_roles)
        .map_err(|error| storage_error("count action tree root children", error))?;
    let observed_agent = observed_agent
        .as_ref()
        .map(action_json)
        .unwrap_or_else(|| "null".to_string());
    Ok(format!(
        "{{\"root\":{{\"id\":{},\"observed_agent\":{},\"has_children\":{},\"child_count\":{}}},\"summary\":{}}}",
        json::string(NODE_ID_AGENT),
        observed_agent,
        bool_json(root_child_count > 0),
        json::number(root_child_count),
        summary_json(summary)
    ))
}

pub(super) fn action_tree_children_json(
    _storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let display_roles = display_parent_role_names();
    let (mut rows, total) = if parent_id == NODE_ID_AGENT {
        let root_roles = ROOT_LINK_ROLES
            .iter()
            .map(|role| role.as_str())
            .collect::<Vec<_>>();
        let result = storage
            .semantic_action_display_root_children_page(trace_id, &display_roles, &root_roles, page)
            .map_err(|error| storage_error("read action tree root children", error))?;
        let rows = result
            .rows
            .into_iter()
            .map(|row| DisplayChild {
                action: row.action,
                link: row.root_link,
                child_count: row.child_count,
            })
            .collect::<Vec<DisplayChild>>();
        (rows, result.total_count)
    } else {
        let result = storage
            .semantic_action_children_page(
                trace_id,
                parent_id,
                &display_roles,
                &display_roles,
                page,
            )
            .map_err(|error| storage_error("read action tree children", error))?;
        let rows = result
            .rows
            .into_iter()
            .map(|row| DisplayChild {
                action: row.action,
                link: Some(row.link),
                child_count: row.child_count,
            })
            .collect::<Vec<DisplayChild>>();
        (rows, result.total_count)
    };
    for row in &mut rows {
        LlmCallDisplay::normalize_one(storage, &mut row.action)?;
        LlmToolDisplay::normalize_one(storage, &mut row.action);
    }
    let actions = rows
        .iter()
        .map(|row| action_json_lite(&row.action))
        .collect::<Vec<_>>();
    let links = rows
        .iter()
        .filter_map(|row| row.link.as_ref().map(link_json))
        .collect::<Vec<_>>();
    let child_state = rows
        .iter()
        .map(|row| child_state_json(&row.action, row.child_count))
        .collect::<Vec<_>>();
    Ok(format!(
        "{{\"parent\":{},\"offset\":{},\"limit\":{},\"total\":{},\"next_offset\":{},\"has_more\":{},\"actions\":[{}],\"links\":[{}],\"child_state\":[{}]}}",
        json::string(parent_id),
        json::number(page.offset),
        json::number(page.limit),
        json::number(total),
        next_offset_json(page, total),
        bool_json(has_more_children(page, total)),
        actions.join(","),
        links.join(","),
        child_state.join(",")
    ))
}

pub(super) fn action_tree_llm_nav_json(
    _storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    mode: LlmNavMode,
    after_action_id: Option<&str>,
) -> Result<String, String> {
    let display_roles = display_parent_role_names();
    let after = match mode {
        LlmNavMode::First => None,
        LlmNavMode::Next => after_action_id,
    };
    let path = storage
        .semantic_action_display_path_to_kind(trace_id, &display_roles, "llm.call", after)
        .map_err(|error| storage_error("navigate action tree LLM calls", error))?;
    let rows = path
        .as_ref()
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    let parent_action_id =
                        entry.parent_action_id.as_deref().unwrap_or(NODE_ID_AGENT);
                    format!(
                        "{{\"parent_action_id\":{},\"action_id\":{},\"offset\":{},\"kind\":{}}}",
                        json::string(parent_action_id),
                        json::string(&entry.action_id),
                        json::number(entry.offset),
                        json::string(entry.kind.as_str())
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    Ok(format!(
        "{{\"mode\":{},\"found\":{},\"path\":[{}]}}",
        json::string(mode.as_str()),
        bool_json(path.is_some()),
        rows
    ))
}

pub(super) fn action_detail_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    action_id: &str,
) -> Result<String, String> {
    let mut action = storage
        .semantic_action_by_id(trace_id, action_id)
        .map_err(|error| storage_error("read semantic action", error))?
        .ok_or_else(|| format!("semantic action {action_id} not found"))?;
    LlmCallDisplay::normalize_one(storage, &mut action)?;
    LlmToolDisplay::normalize_one(storage, &mut action);
    Ok(action_json(&action))
}

pub(super) fn file_path_set_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    action_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let path_page = storage
        .file_path_set_paths_page(trace_id, action_id, page.offset, page.limit)
        .map_err(|error| storage_error("read file path set", error))?;
    Ok(match path_page {
        Some(path_page) => file_path_set_page_json(path_page, page),
        None => format!(
            "{{\"path_set\":null,\"offset\":{},\"limit\":{},\"total\":0,\"next_offset\":null,\"has_more\":false,\"paths\":[]}}",
            json::number(page.offset),
            json::number(page.limit)
        ),
    })
}

pub(super) fn llm_request_content_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    action_id: &str,
    max_bytes: usize,
) -> Result<String, String> {
    let content = storage
        .llm_request_content_page(trace_id, action_id, max_bytes)
        .map_err(|error| storage_error("read LLM request content", error))?;
    Ok(match content {
        Some(content) => format!("{{\"content\":{}}}", llm_request_content_page_json(content)),
        None => "{\"content\":null}".to_string(),
    })
}

pub(super) fn llm_request_lineage_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    action_id: &str,
) -> Result<String, String> {
    let lineage = storage
        .llm_request_lineage(trace_id, action_id)
        .map_err(|error| storage_error("read LLM request lineage", error))?;
    let forks = storage
        .llm_request_forks(trace_id, action_id)
        .map_err(|error| storage_error("read LLM request forks", error))?;
    Ok(format!(
        "{{\"lineage\":{},\"forks\":[{}]}}",
        lineage
            .as_ref()
            .map(llm_request_lineage_row_json)
            .unwrap_or_else(|| "null".to_string()),
        forks
            .iter()
            .map(llm_request_lineage_row_json)
            .collect::<Vec<_>>()
            .join(",")
    ))
}

pub(super) fn llm_request_trajectory_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    trajectory_id: &str,
) -> Result<String, String> {
    let nodes = storage
        .llm_request_trajectory(trace_id, trajectory_id)
        .map_err(|error| storage_error("read LLM request trajectory", error))?;
    Ok(format!(
        "{{\"trajectory_id\":{},\"nodes\":[{}]}}",
        json::string(trajectory_id),
        nodes
            .iter()
            .map(llm_request_lineage_row_json)
            .collect::<Vec<_>>()
            .join(",")
    ))
}

pub(super) fn mcp_jsonrpc_content_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    action_id: &str,
    max_bytes: usize,
) -> Result<String, String> {
    let content = storage
        .mcp_jsonrpc_content_page(trace_id, action_id, max_bytes)
        .map_err(|error| storage_error("read MCP JSON-RPC content", error))?;
    Ok(match content {
        Some(content) => format!("{{\"content\":{}}}", mcp_jsonrpc_content_page_json(content)),
        None => "{\"content\":null}".to_string(),
    })
}

pub(super) fn llm_request_content_node_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    action_id: &str,
    query: LlmRequestContentNodeQuery,
) -> Result<String, String> {
    let content = storage
        .llm_request_content_page(trace_id, action_id, usize::MAX)
        .map_err(|error| storage_error("read LLM request content node", error))?;
    let Some(content) = content else {
        return Ok("{\"content\":null}".to_string());
    };
    let body = serde_json::from_str::<Value>(&content.body_json)
        .map_err(|error| format!("parse canonical LLM request body failed: {error}"))?;
    let node = body
        .pointer(&query.pointer)
        .ok_or_else(|| format!("JSON Pointer not found: {}", query.pointer))?;
    let node_json = llm_request_node_json(node, &query);
    serde_json::to_string(&json_value!({
        "content": {
            "action_id": content.action_id,
            "format_version": content.format_version,
            "canonical_body_bytes": content.canonical_body_bytes,
            "pointer": query.pointer,
            "node": node_json,
        }
    }))
    .map_err(|error| format!("serialize LLM request content node failed: {error}"))
}

fn llm_request_node_json(node: &Value, query: &LlmRequestContentNodeQuery) -> Value {
    match node {
        Value::Object(object) => {
            let total = object.len();
            let children = object
                .iter()
                .skip(query.offset)
                .take(query.limit)
                .map(|(key, value)| {
                    llm_request_child_json(key, &json_pointer_child(&query.pointer, key), value)
                })
                .collect::<Vec<_>>();
            branch_node_json("object", total, children, query)
        }
        Value::Array(array) => {
            let total = array.len();
            let children = array
                .iter()
                .enumerate()
                .skip(query.offset)
                .take(query.limit)
                .map(|(index, value)| {
                    let token = index.to_string();
                    llm_request_child_json(
                        &token,
                        &json_pointer_child(&query.pointer, &token),
                        value,
                    )
                })
                .collect::<Vec<_>>();
            branch_node_json("array", total, children, query)
        }
        Value::Null => json_value!({
            "type": "null",
            "expandable": false,
            "value": Value::Null,
        }),
        Value::Bool(value) => leaf_node_json("boolean", Value::Bool(*value)),
        Value::Number(value) => leaf_node_json("number", Value::Number(value.clone())),
        Value::String(value) => leaf_node_json("string", Value::String(value.clone())),
    }
}

fn branch_node_json(
    kind: &str,
    total: usize,
    children: Vec<Value>,
    query: &LlmRequestContentNodeQuery,
) -> Value {
    let returned = children.len();
    let next_offset = query.offset.saturating_add(returned);
    let has_more = next_offset < total;
    json_value!({
        "type": kind,
        "expandable": true,
        "total_children": total,
        "offset": query.offset,
        "limit": query.limit,
        "next_offset": has_more.then_some(next_offset),
        "has_more": has_more,
        "children": children,
    })
}

fn leaf_node_json(kind: &str, value: Value) -> Value {
    json_value!({
        "type": kind,
        "expandable": false,
        "value": value,
    })
}

fn llm_request_child_json(token: &str, pointer: &str, value: &Value) -> Value {
    json_value!({
        "token": token,
        "pointer": pointer,
        "type": json_value_type(value),
        "expandable": matches!(value, Value::Object(_) | Value::Array(_)),
        "child_count": json_value_child_count(value),
    })
}

fn json_value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn json_value_child_count(value: &Value) -> Option<usize> {
    match value {
        Value::Array(array) => Some(array.len()),
        Value::Object(object) => Some(object.len()),
        _ => None,
    }
}

fn json_pointer_child(parent: &str, token: &str) -> String {
    format!("{parent}/{}", token.replace('~', "~0").replace('/', "~1"))
}

fn display_parent_role_names() -> Vec<&'static str> {
    DISPLAY_PARENT_ROLES
        .iter()
        .map(|role| role.as_str())
        .collect()
}

fn has_more_children(page: SemanticActionChildPageQuery, total: usize) -> bool {
    page.offset.saturating_add(page.limit) < total
}

fn next_offset_json(page: SemanticActionChildPageQuery, total: usize) -> String {
    if has_more_children(page, total) {
        json::number(page.offset.saturating_add(page.limit))
    } else {
        "null".to_string()
    }
}

fn file_path_set_page_json(
    path_page: FilePathSetPathPage,
    page: SemanticActionChildPageQuery,
) -> String {
    let paths = path_page
        .paths
        .iter()
        .map(file_path_set_path_json)
        .collect::<Vec<_>>();
    format!(
        "{{\"path_set\":{{\"id\":{},\"action_id\":{},\"state\":{},\"unique_path_count\":{},\"stored_path_count\":{},\"chunking_scheme\":{}}},\"offset\":{},\"limit\":{},\"total\":{},\"next_offset\":{},\"has_more\":{},\"paths\":[{}]}}",
        json::string(&path_page.path_set_id),
        json::string(&path_page.action_id),
        json::string(path_page.state.as_str()),
        json::number(path_page.unique_path_count),
        json::number(path_page.stored_path_count),
        json::string(&path_page.chunking_scheme),
        json::number(page.offset),
        json::number(page.limit),
        json::number(path_page.total_count),
        next_offset_json(page, path_page.total_count),
        bool_json(has_more_children(page, path_page.total_count)),
        paths.join(",")
    )
}

fn file_path_set_path_json(path: &FilePathSetPath) -> String {
    format!(
        "{{\"path_id\":{},\"path\":{}}}",
        json::number(path.path_id),
        json::string(&path.path)
    )
}

fn llm_request_content_page_json(content: LlmRequestContentPage) -> String {
    format!(
        "{{\"action_id\":{},\"format_version\":{},\"canonical_body_bytes\":{},\"returned_bytes\":{},\"truncated\":{},\"body_json\":{}}}",
        json::string(&content.action_id),
        json::number(content.format_version),
        json::number(content.canonical_body_bytes),
        json::number(content.returned_bytes),
        bool_json(content.truncated),
        json::string(&content.body_json)
    )
}

fn llm_request_lineage_row_json(lineage: &LlmRequestLineage) -> String {
    format!(
        "{{\"action_id\":{},\"trajectory_id\":{},\"parent_action_id\":{},\"forked_from_action_id\":{},\"trajectory_position\":{},\"transition\":{},\"start_reason\":{},\"inference_version\":{}}}",
        json::string(&lineage.action_id),
        json::string(&lineage.trajectory_id),
        lineage
            .parent_action_id
            .as_deref()
            .map(json::string)
            .unwrap_or_else(|| "null".to_string()),
        lineage
            .forked_from_action_id
            .as_deref()
            .map(json::string)
            .unwrap_or_else(|| "null".to_string()),
        json::number(lineage.trajectory_position),
        json::string(lineage.transition.as_str()),
        json::string(lineage.start_reason.as_str()),
        json::number(lineage.inference_version),
    )
}

fn mcp_jsonrpc_content_page_json(content: McpJsonRpcContentPage) -> String {
    format!(
        "{{\"action_id\":{},\"format_version\":{},\"canonical_json_hash\":{},\"canonical_json_bytes\":{},\"returned_bytes\":{},\"truncated\":{},\"canonical_json\":{}}}",
        json::string(&content.action_id),
        json::number(content.format_version),
        json::string(&content.canonical_json_hash),
        json::number(content.canonical_json_bytes),
        json::number(content.returned_bytes),
        bool_json(content.truncated),
        json::string(&content.canonical_json)
    )
}

fn child_state_json(action: &SemanticAction, child_count: usize) -> String {
    format!(
        "{{\"id\":{},\"has_children\":{},\"child_count\":{}}}",
        json::string(&action.action_id),
        bool_json(child_count > 0),
        json::number(child_count)
    )
}

fn summary_json(summary: SemanticActionSummary) -> String {
    format!(
        "{{\"actions\":{},\"links\":{},\"roots\":{}}}",
        json::number(summary.actions),
        json::number(summary.links),
        json::number(summary.roots)
    )
}

fn storage_error(stage: &str, error: StorageError) -> String {
    format!("{} failed: {}: {}", stage, error.stage, error.message)
}

/// Server-provided timeline axis end: `max(last_observed_at, server_now)` so
/// open intervals and unfinished actions extend with server time, not the
/// browser clock. Completed traces keep their last observed end; extending
/// those to "now" would render a huge empty tail after the trace ended.
fn axis_end_unix_nanos(actions: &[SemanticAction], intervals: &[AgentIdleInterval]) -> String {
    let now = SystemTime::now();
    let end = latest_observed_at(actions, intervals).unwrap_or(now);
    let axis_end = if has_live_item(actions, intervals) {
        end.max(now)
    } else {
        end
    };
    json::time_nanos(axis_end)
}

fn has_live_item(actions: &[SemanticAction], intervals: &[AgentIdleInterval]) -> bool {
    actions.iter().any(|action| action.end_time.is_none())
        || intervals.iter().any(|interval| interval.end_time.is_none())
}

fn latest_observed_at(
    actions: &[SemanticAction],
    intervals: &[AgentIdleInterval],
) -> Option<SystemTime> {
    let mut latest: Option<SystemTime> = None;
    for time in actions
        .iter()
        .flat_map(|action| {
            [Some(action.start_time), action.end_time]
                .into_iter()
                .flatten()
        })
        .chain(intervals.iter().flat_map(|interval| {
            [Some(interval.start_time), interval.end_time]
                .into_iter()
                .flatten()
        }))
    {
        latest = Some(latest.map_or(time, |current| current.max(time)));
    }
    latest
}

fn bool_json(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
