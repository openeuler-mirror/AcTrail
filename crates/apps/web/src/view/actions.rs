//! Semantic action details and bounded content pages for the web UI.

use std::path::Path;

use model_core::ids::TraceId;
use semantic_action::{
    FilePathSetPath, FilePathSetPathPage, LlmRequestContentPage, LlmRequestLineage,
    McpJsonRpcContentPage, SemanticAction,
};
use serde_json::{Value, json as json_value};
use storage_core::{
    SemanticActionChildPageQuery, SemanticActionSummary, StorageBackend, StorageError,
};

use super::LlmNavMode;
use super::action_tree_roles::{DISPLAY_PARENT_ROLES, NODE_ID_AGENT, ROOT_LINK_ROLES};
use crate::json;

#[path = "actions/serialization.rs"]
mod serialization;

use serialization::link_json;
pub(super) use serialization::{action_json, action_json_lite};

#[path = "actions/llm_call_display.rs"]
mod llm_call_display;
#[path = "actions/llm_tool_display.rs"]
mod llm_tool_display;
use llm_call_display::LlmCallDisplay;
use llm_tool_display::LlmToolDisplay;

struct DisplayChild {
    action: SemanticAction,
    link: Option<semantic_action::SemanticActionLink>,
    child_count: usize,
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
    query: storage_core::ContentNodeQuery,
) -> Result<String, String> {
    let content = storage
        .llm_request_content_node(trace_id, action_id, &query)
        .map_err(|error| storage_error("read LLM request content node", error))?;
    let Some(content) = content else {
        return Ok("{\"content\":null}".into());
    };
    serde_json::to_string(&json_value!({
        "content": {
            "action_id": content.action_id,
            "format_version": content.format_version,
            "canonical_body_bytes": content.canonical_body_bytes,
            "pointer": content.pointer,
            "node": ContentNodeJson::render(content.node),
        }
    }))
    .map_err(|error| format!("serialize LLM request content node failed: {error}"))
}

struct ContentNodeJson;

impl ContentNodeJson {
    fn render(node: storage_core::ContentNode) -> Value {
        use storage_core::{ContentNode, ContentNodeKind};
        match node {
            ContentNode::Branch {
                kind,
                total_children,
                offset,
                limit,
                next_offset,
                children,
            } => {
                let children = children.into_iter().map(|child| json_value!({
                    "token":child.token, "pointer":child.pointer, "type":child.kind.as_str(),
                    "expandable":matches!(child.kind, ContentNodeKind::Array | ContentNodeKind::Object),
                    "child_count":child.child_count,
                })).collect::<Vec<_>>();
                json_value!({"type":kind.as_str(), "expandable":true, "total_children":total_children,
                    "offset":offset, "limit":limit, "next_offset":next_offset,
                    "has_more":next_offset.is_some(), "children":children})
            }
            ContentNode::Leaf {
                value,
                value_bytes,
                returned_bytes,
                offset,
                next_offset,
                truncated,
            } => {
                let kind = match &value {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                };
                json_value!({"type":kind, "expandable":false, "value":value,
                    "offset":offset, "next_offset":next_offset, "has_more":next_offset.is_some(),
                    "value_bytes":value_bytes, "returned_bytes":returned_bytes, "truncated":truncated})
            }
        }
    }
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

fn bool_json(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
