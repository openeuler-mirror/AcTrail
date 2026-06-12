//! Semantic action tree JSON for the web UI.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionLink};
use sqlite_storage::SqliteStorage;
use sqlite_storage::semantic_actions::SemanticActionSummary;

use super::action_tree_projection::{
    ActionDisplayProjection, DisplayChild, display_parent_role_strs, root_link_role_strs,
};
use super::projection_cache::cached_action_display_projection;
use crate::json;

const NODE_ID_AGENT: &str = "agent-process";

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
    storage_path: &std::path::Path,
    storage: &mut SqliteStorage,
    trace_id: TraceId,
) -> Result<String, String> {
    let projection = load_cached_projection(storage_path, storage, trace_id)?;
    let roots = projection
        .root_action_ids
        .iter()
        .map(|action_id| json::string(action_id))
        .collect::<Vec<_>>();
    let action_rows = projection
        .actions
        .iter()
        .map(action_json_lite)
        .collect::<Vec<_>>();
    let link_rows = projection.links.iter().map(link_json).collect::<Vec<_>>();

    let mut output = String::from("{");
    json::field(&mut output, "roots", &format!("[{}]", roots.join(",")));
    output.push(',');
    json::field(
        &mut output,
        "actions",
        &format!("[{}]", action_rows.join(",")),
    );
    output.push(',');
    json::field(&mut output, "links", &format!("[{}]", link_rows.join(",")));
    output.push('}');
    Ok(output)
}

pub(super) fn action_tree_root_json(
    storage_path: &std::path::Path,
    storage: &mut SqliteStorage,
    trace_id: TraceId,
) -> Result<String, String> {
    let summary = storage.semantic_action_summary(trace_id).map_err(|error| {
        format!(
            "read semantic action summary failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let observed_agent = storage
        .observed_agent_semantic_action(trace_id)
        .map_err(|error| {
            format!(
                "read observed agent action failed: {}: {}",
                error.stage, error.message
            )
        })?;
    let child_count = root_child_count(storage, trace_id, &summary, observed_agent.as_ref())?;

    let mut root = String::from("{");
    json::field(&mut root, "id", &json::string(NODE_ID_AGENT));
    root.push(',');
    json::field(
        &mut root,
        "observed_agent",
        &observed_agent
            .as_ref()
            .map(action_json)
            .unwrap_or_else(|| "null".to_string()),
    );
    root.push(',');
    json::field(&mut root, "has_children", bool_json(child_count > 0));
    root.push(',');
    json::field(&mut root, "child_count", &json::number(child_count));
    root.push('}');

    let mut output = String::from("{");
    json::field(&mut output, "root", &root);
    output.push(',');
    json::field(&mut output, "summary", &summary_json(summary));
    output.push('}');
    let _ = storage_path;
    Ok(output)
}

pub(super) fn action_tree_children_json(
    storage_path: &std::path::Path,
    storage: &mut SqliteStorage,
    trace_id: TraceId,
    parent_id: &str,
) -> Result<String, String> {
    let rows = if parent_id == NODE_ID_AGENT {
        load_agent_root_children(storage_path, storage, trace_id)?
    } else {
        load_display_children(storage, trace_id, parent_id)?
    };
    let action_rows = rows
        .iter()
        .map(|row| action_json_lite(&row.action))
        .collect::<Vec<_>>();
    let link_rows = rows
        .iter()
        .filter_map(|row| row.link.as_ref())
        .map(link_json)
        .collect::<Vec<_>>();
    let child_state_rows = rows
        .iter()
        .map(|row| child_state_json(&row.action, row.child_count))
        .collect::<Vec<_>>();

    let mut output = String::from("{");
    json::field(&mut output, "parent", &json::string(parent_id));
    output.push(',');
    json::field(
        &mut output,
        "actions",
        &format!("[{}]", action_rows.join(",")),
    );
    output.push(',');
    json::field(&mut output, "links", &format!("[{}]", link_rows.join(",")));
    output.push(',');
    json::field(
        &mut output,
        "child_state",
        &format!("[{}]", child_state_rows.join(",")),
    );
    output.push('}');
    Ok(output)
}

fn load_cached_projection(
    storage_path: &std::path::Path,
    storage: &mut SqliteStorage,
    trace_id: TraceId,
) -> Result<std::sync::Arc<ActionDisplayProjection>, String> {
    cached_action_display_projection(storage_path, trace_id, || {
        ActionDisplayProjection::load(storage, trace_id)
    })
}

fn root_child_count(
    storage: &mut SqliteStorage,
    trace_id: TraceId,
    summary: &SemanticActionSummary,
    observed_agent: Option<&SemanticAction>,
) -> Result<usize, String> {
    if let Some(agent) = observed_agent {
        return storage
            .semantic_action_child_count(trace_id, &agent.action_id, root_link_role_strs())
            .map_err(|error| {
                format!(
                    "count agent root children failed: {}: {}",
                    error.stage, error.message
                )
            });
    }
    Ok(summary.roots)
}

fn load_agent_root_children(
    storage_path: &std::path::Path,
    storage: &mut SqliteStorage,
    trace_id: TraceId,
) -> Result<Vec<DisplayChild>, String> {
    let observed_agent = storage
        .observed_agent_semantic_action(trace_id)
        .map_err(|error| {
            format!(
                "read observed agent action failed: {}: {}",
                error.stage, error.message
            )
        })?;
    if let Some(agent) = observed_agent {
        let roles = display_parent_role_strs();
        let rows = storage
            .semantic_action_children(trace_id, &agent.action_id, root_link_role_strs(), roles)
            .map_err(|error| {
                format!(
                    "list agent root children failed: {}: {}",
                    error.stage, error.message
                )
            })?;
        if !rows.is_empty() {
            return Ok(rows
                .into_iter()
                .map(|row| DisplayChild {
                    action: row.action,
                    link: Some(row.link),
                    child_count: row.child_count,
                })
                .collect());
        }
    }
    let projection = load_cached_projection(storage_path, storage, trace_id)?;
    Ok(projection.root_children())
}

fn load_display_children(
    storage: &mut SqliteStorage,
    trace_id: TraceId,
    parent_id: &str,
) -> Result<Vec<DisplayChild>, String> {
    let roles = display_parent_role_strs();
    let mut rows = storage
        .semantic_action_children(trace_id, parent_id, roles, roles)
        .map_err(|error| {
            format!(
                "list semantic action children failed: {}: {}",
                error.stage, error.message
            )
        })?
        .into_iter()
        .map(|row| DisplayChild {
            action: row.action,
            link: Some(row.link),
            child_count: row.child_count,
        })
        .collect::<Vec<_>>();
    let parent = storage
        .semantic_action_by_id(trace_id, parent_id)
        .map_err(|error| {
            format!(
                "read parent semantic action failed: {}: {}",
                error.stage, error.message
            )
        })?;
    if parent
        .as_ref()
        .is_some_and(|action| action.kind == SemanticActionKind::CommandInvocation)
    {
        let parent = parent.expect("parent action was just checked");
        let linked_ids = rows
            .iter()
            .map(|row| row.action.action_id.clone())
            .collect::<BTreeSet<_>>();
        let fallback = storage
            .semantic_action_command_fallback_children(trace_id, &parent, roles)
            .map_err(|error| {
                format!(
                    "list command fallback children failed: {}: {}",
                    error.stage, error.message
                )
            })?;
        for action in fallback {
            if linked_ids.contains(&action.action_id) {
                continue;
            }
            let child_count = storage
                .semantic_action_child_count(trace_id, &action.action_id, roles)
                .map_err(|error| {
                    format!(
                        "count fallback child actions failed: {}: {}",
                        error.stage, error.message
                    )
                })?;
            rows.push(DisplayChild {
                action,
                link: None,
                child_count,
            });
        }
    }
    rows.sort_by(|left, right| {
        (left.action.start_time, left.action.action_id.as_str())
            .cmp(&(right.action.start_time, right.action.action_id.as_str()))
    });
    Ok(rows)
}

pub(super) fn action_json(action: &SemanticAction) -> String {
    render_action_json(action, false)
}

pub(super) fn action_json_lite(action: &SemanticAction) -> String {
    render_action_json(action, true)
}

fn render_action_json(action: &SemanticAction, lite: bool) -> String {
    let attributes = if lite {
        lite_attributes(&action.attributes)
    } else {
        action.attributes.clone()
    };
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::string(&action.action_id));
    output.push(',');
    json::field(&mut output, "kind", &json::string(action.kind.as_str()));
    output.push(',');
    json::field(&mut output, "title", &json::string(&action.title));
    output.push(',');
    json::field(&mut output, "start_time", &json::time(action.start_time));
    output.push(',');
    json::field(
        &mut output,
        "start_time_unix_nanos",
        &json::time_nanos(action.start_time),
    );
    output.push(',');
    json::field(
        &mut output,
        "end_time",
        &action
            .end_time
            .map(json::time)
            .unwrap_or_else(|| "null".to_string()),
    );
    output.push(',');
    json::field(
        &mut output,
        "end_time_unix_nanos",
        &json::optional_time_nanos(action.end_time),
    );
    output.push(',');
    json::field(
        &mut output,
        "duration",
        &action
            .end_time
            .and_then(|end| end.duration_since(action.start_time).ok())
            .map(|d| json::string(&json::duration_micros(d.as_micros() as u64)))
            .unwrap_or_else(|| "null".to_string()),
    );
    output.push(',');
    json::field(&mut output, "process", &json::process(&action.process));
    output.push(',');
    json::field(&mut output, "status", &json::string(action.status.as_str()));
    output.push(',');
    json::field(
        &mut output,
        "completeness",
        &json::string(action.completeness.as_str()),
    );
    output.push(',');
    json::field(
        &mut output,
        "confidence_millis",
        &json::optional_number(action.confidence_millis),
    );
    output.push(',');
    json::field(&mut output, "attributes", &json::map(&attributes));
    output.push(',');
    json::field(
        &mut output,
        "evidence",
        &if lite {
            "[]".to_string()
        } else {
            evidence_json(&action.evidence)
        },
    );
    output.push('}');
    output
}

fn lite_attributes(
    attributes: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    attributes
        .iter()
        .filter(|(key, _)| !is_heavy_attribute(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn is_heavy_attribute(key: &str) -> bool {
    HEAVY_ATTRIBUTE_KEYS.contains(&key)
        || HEAVY_ATTRIBUTE_SUFFIXES
            .iter()
            .any(|suffix| key.ends_with(suffix))
}

fn link_json(link: &SemanticActionLink) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "parent", &json::string(&link.parent_action_id));
    output.push(',');
    json::field(&mut output, "child", &json::string(&link.child_action_id));
    output.push(',');
    json::field(&mut output, "role", &json::string(link.role.as_str()));
    output.push(',');
    json::field(
        &mut output,
        "confidence",
        &json::string(link.confidence.as_str()),
    );
    output.push(',');
    json::field(&mut output, "attributes", &json::map(&link.attributes));
    output.push(',');
    json::field(
        &mut output,
        "evidence",
        &if link.evidence.is_empty() {
            "[]".to_string()
        } else {
            evidence_json(&link.evidence)
        },
    );
    output.push('}');
    output
}

fn evidence_json(evidence: &[semantic_action::SemanticEvidence]) -> String {
    let rows = evidence
        .iter()
        .map(|evidence| {
            let mut output = String::from("{");
            json::field(&mut output, "kind", &json::string(evidence.kind.as_str()));
            output.push(',');
            json::field(&mut output, "id", &json::number(evidence.id));
            output.push(',');
            json::field(&mut output, "role", &json::string(&evidence.role));
            output.push('}');
            output
        })
        .collect::<Vec<_>>();
    format!("[{}]", rows.join(","))
}

fn child_state_json(action: &SemanticAction, child_count: usize) -> String {
    let has_children = child_count > 0 || !action.evidence.is_empty();
    let mut output = String::from("{");
    json::field(&mut output, "id", &json::string(&action.action_id));
    output.push(',');
    json::field(&mut output, "has_children", bool_json(has_children));
    output.push(',');
    json::field(&mut output, "child_count", &json::number(child_count));
    output.push('}');
    output
}

fn summary_json(summary: SemanticActionSummary) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "actions", &json::number(summary.actions));
    output.push(',');
    json::field(&mut output, "links", &json::number(summary.links));
    output.push(',');
    json::field(&mut output, "roots", &json::number(summary.roots));
    output.push('}');
    output
}

fn bool_json(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}
