//! Semantic action tree JSON for the web UI.

use std::path::Path;

use model_core::ids::TraceId;
use semantic_action::{
    FilePathSetPath, FilePathSetPathPage, LlmRequestContentPage, SemanticAction,
    SemanticActionLink, SemanticEvidence,
};
use storage_core::{
    SemanticActionChildPageQuery, SemanticActionSummary, StorageBackend, StorageError,
};

use super::action_tree_projection::{ActionDisplayProjection, DisplayChild, ROOT_PARENT_ID};
use super::action_tree_roles::NODE_ID_AGENT;
use super::projection_cache;
use crate::json;

const HEAVY_ATTRIBUTE_KEYS: &[&str] = &[
    "http.request.body_text",
    "http.request.body_json",
    "http.response.body_text",
    "http.response.body_json",
];
const HEAVY_ATTRIBUTE_SUFFIXES: &[&str] = &[
    ".payload_text",
    ".body_text",
    ".body_json",
    ".output_text",
    ".content_text",
    ".reasoning_text",
];

pub(super) fn action_tree_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
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
        .map(action_json)
        .collect::<Vec<_>>();
    let links = projection.links.iter().map(link_json).collect::<Vec<_>>();
    Ok(format!(
        "{{\"roots\":[{}],\"actions\":[{}],\"links\":[{}]}}",
        roots.join(","),
        actions.join(","),
        links.join(",")
    ))
}

pub(super) fn action_tree_root_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let projection =
        projection_cache::cached_action_display_projection(storage_path, trace_id, || {
            ActionDisplayProjection::load(storage, trace_id)
        })?;
    let summary = storage
        .semantic_action_summary(trace_id)
        .map_err(|error| storage_error("read semantic action summary", error))?;
    let observed_agent = storage
        .observed_agent_semantic_action(trace_id)
        .map_err(|error| storage_error("read observed agent action", error))?;
    let root_child_count = projection.child_count(ROOT_PARENT_ID);
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
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let projection =
        projection_cache::cached_action_display_projection(storage_path, trace_id, || {
            ActionDisplayProjection::load(storage, trace_id)
        })?;
    let (rows, total) = load_child_page(&projection, parent_id, page);
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

pub(super) fn action_detail_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    action_id: &str,
) -> Result<String, String> {
    let action = storage
        .semantic_action_by_id(trace_id, action_id)
        .map_err(|error| storage_error("read semantic action", error))?
        .ok_or_else(|| format!("semantic action {action_id} not found"))?;
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

fn load_child_page(
    projection: &ActionDisplayProjection,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> (Vec<DisplayChild>, usize) {
    let parent_id = if parent_id == NODE_ID_AGENT {
        ROOT_PARENT_ID
    } else {
        parent_id
    };
    projection.children_page(parent_id, page.offset, page.limit)
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

pub(super) fn action_json(action: &SemanticAction) -> String {
    render_action_json(action, false)
}

pub(super) fn action_json_lite(action: &SemanticAction) -> String {
    render_action_json(action, true)
}

fn render_action_json(action: &SemanticAction, lite: bool) -> String {
    let attributes = if lite {
        action
            .attributes
            .iter()
            .filter(|(key, _)| !is_heavy_attribute(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    } else {
        action.attributes.clone()
    };
    let evidence = if lite {
        "[]".to_string()
    } else {
        evidence_json(&action.evidence)
    };
    format!(
        "{{\"id\":{},\"kind\":{},\"title\":{},\"start_time\":{},\"start_time_unix_nanos\":{},\"end_time\":{},\"end_time_unix_nanos\":{},\"duration\":{},\"process\":{},\"status\":{},\"completeness\":{},\"confidence_millis\":{},\"attributes\":{},\"evidence\":{}}}",
        json::string(&action.action_id),
        json::string(action.kind.as_str()),
        json::string(&action.title),
        json::time(action.start_time),
        json::time_nanos(action.start_time),
        action
            .end_time
            .map(json::time)
            .unwrap_or_else(|| "null".to_string()),
        json::optional_time_nanos(action.end_time),
        action
            .end_time
            .and_then(|end| end.duration_since(action.start_time).ok())
            .map(|duration| json::string(&json::duration_micros(duration.as_micros() as u64)))
            .unwrap_or_else(|| "null".to_string()),
        json::process(&action.process),
        json::string(action.status.as_str()),
        json::string(action.completeness.as_str()),
        json::optional_number(action.confidence_millis),
        json::map(&attributes),
        evidence
    )
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
        "{{\"action_id\":{},\"format_version\":{},\"canonical_body_hash\":{},\"canonical_body_bytes\":{},\"returned_bytes\":{},\"truncated\":{},\"body_json\":{}}}",
        json::string(&content.action_id),
        json::number(content.format_version),
        json::string(&content.canonical_body_hash),
        json::number(content.canonical_body_bytes),
        json::number(content.returned_bytes),
        bool_json(content.truncated),
        json::string(&content.body_json)
    )
}

fn is_heavy_attribute(key: &str) -> bool {
    HEAVY_ATTRIBUTE_KEYS.contains(&key)
        || HEAVY_ATTRIBUTE_SUFFIXES
            .iter()
            .any(|suffix| key.ends_with(suffix))
}

fn link_json(link: &SemanticActionLink) -> String {
    let evidence = if link.evidence.is_empty() {
        "[]".to_string()
    } else {
        evidence_json(&link.evidence)
    };
    format!(
        "{{\"parent\":{},\"child\":{},\"role\":{},\"confidence\":{},\"valid\":{},\"attributes\":{},\"evidence\":{}}}",
        json::string(&link.parent_action_id),
        json::string(&link.child_action_id),
        json::string(link.role.as_str()),
        json::string(link.confidence.as_str()),
        json::boolean(link.valid),
        json::map(&link.attributes),
        evidence
    )
}

fn evidence_json(evidence: &[SemanticEvidence]) -> String {
    let rows = evidence
        .iter()
        .map(|evidence| {
            format!(
                "{{\"kind\":{},\"id\":{},\"role\":{}}}",
                json::string(evidence.kind.as_str()),
                json::number(evidence.id),
                json::string(&evidence.role)
            )
        })
        .collect::<Vec<_>>();
    format!("[{}]", rows.join(","))
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
