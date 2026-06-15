//! Semantic action tree JSON for the web UI.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionLink, SemanticEvidence};
use storage_core::{
    SemanticActionChildPageQuery, SemanticActionChildRow, SemanticActionDisplayRootChildRow,
    SemanticActionSummary, StorageBackend, StorageError,
};

use super::action_tree_projection::{ActionDisplayProjection, DisplayChild};
use super::action_tree_roles::{DISPLAY_PARENT_ROLE_STRS, NODE_ID_AGENT, ROOT_LINK_ROLE_STRS};
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
    ".tool_calls_json",
];

pub(super) fn action_tree_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let projection = ActionDisplayProjection::load(storage, trace_id)?;
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
    let links = projection.links.iter().map(link_json).collect::<Vec<_>>();
    Ok(format!(
        "{{\"roots\":[{}],\"actions\":[{}],\"links\":[{}]}}",
        roots.join(","),
        actions.join(","),
        links.join(",")
    ))
}

pub(super) fn action_tree_root_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let summary = storage
        .semantic_action_summary(trace_id)
        .map_err(|error| storage_error("read semantic action summary", error))?;
    let observed_agent = storage
        .observed_agent_semantic_action(trace_id)
        .map_err(|error| storage_error("read observed agent action", error))?;
    let root_child_count = storage
        .semantic_action_display_root_child_count(trace_id, DISPLAY_PARENT_ROLE_STRS)
        .map_err(|error| storage_error("count display root semantic action children", error))?;
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
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let (rows, total) = load_child_page(storage, trace_id, parent_id, page)?;
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

fn load_child_page(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<(Vec<DisplayChild>, usize), String> {
    if parent_id == NODE_ID_AGENT {
        return load_agent_root_page(storage, trace_id, page);
    }
    let parent = storage
        .semantic_action_by_id(trace_id, parent_id)
        .map_err(|error| storage_error("read parent semantic action", error))?;
    if parent
        .as_ref()
        .is_some_and(|action| action.kind == SemanticActionKind::CommandInvocation)
    {
        let rows = load_command_children(storage, trace_id, parent.expect("checked parent"))?;
        let total = rows.len();
        return Ok((page_rows(rows, page), total));
    }
    let rows = storage
        .semantic_action_children_page(
            trace_id,
            parent_id,
            DISPLAY_PARENT_ROLE_STRS,
            DISPLAY_PARENT_ROLE_STRS,
            page,
        )
        .map_err(|error| storage_error("list semantic action children", error))?;
    Ok((
        rows.rows.into_iter().map(display_child).collect(),
        rows.total_count,
    ))
}

fn load_agent_root_page(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    page: SemanticActionChildPageQuery,
) -> Result<(Vec<DisplayChild>, usize), String> {
    let page = storage
        .semantic_action_display_root_children_page(
            trace_id,
            DISPLAY_PARENT_ROLE_STRS,
            ROOT_LINK_ROLE_STRS,
            page,
        )
        .map_err(|error| storage_error("list display root semantic action children", error))?;
    Ok((
        page.rows.into_iter().map(display_root_child).collect(),
        page.total_count,
    ))
}

fn load_command_children(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    parent: SemanticAction,
) -> Result<Vec<DisplayChild>, String> {
    let mut rows = storage
        .semantic_action_children(
            trace_id,
            &parent.action_id,
            DISPLAY_PARENT_ROLE_STRS,
            DISPLAY_PARENT_ROLE_STRS,
        )
        .map_err(|error| storage_error("list semantic action children", error))?
        .into_iter()
        .map(display_child)
        .collect::<Vec<_>>();
    let linked_ids = rows
        .iter()
        .map(|row| row.action.action_id.clone())
        .collect::<BTreeSet<_>>();
    for action in storage
        .semantic_action_command_fallback_children(trace_id, &parent, DISPLAY_PARENT_ROLE_STRS)
        .map_err(|error| storage_error("list command fallback children", error))?
    {
        if linked_ids.contains(&action.action_id) {
            continue;
        }
        let child_count = storage
            .semantic_action_child_count(trace_id, &action.action_id, DISPLAY_PARENT_ROLE_STRS)
            .map_err(|error| storage_error("count fallback child actions", error))?;
        rows.push(DisplayChild {
            action,
            link: None,
            child_count,
        });
    }
    rows.sort_by(|left, right| {
        (left.action.start_time, left.action.action_id.as_str())
            .cmp(&(right.action.start_time, right.action.action_id.as_str()))
    });
    Ok(rows)
}

fn display_child(row: SemanticActionChildRow) -> DisplayChild {
    DisplayChild {
        action: row.action,
        link: Some(row.link),
        child_count: row.child_count,
    }
}

fn display_root_child(row: SemanticActionDisplayRootChildRow) -> DisplayChild {
    DisplayChild {
        action: row.action,
        link: row.root_link,
        child_count: row.child_count,
    }
}

fn page_rows(rows: Vec<DisplayChild>, page: SemanticActionChildPageQuery) -> Vec<DisplayChild> {
    rows.into_iter()
        .skip(page.offset)
        .take(page.limit)
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
        "{{\"parent\":{},\"child\":{},\"role\":{},\"confidence\":{},\"attributes\":{},\"evidence\":{}}}",
        json::string(&link.parent_action_id),
        json::string(&link.child_action_id),
        json::string(link.role.as_str()),
        json::string(link.confidence.as_str()),
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
