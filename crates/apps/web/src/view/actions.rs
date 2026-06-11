//! Semantic action tree JSON for the web UI.

use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionLink};
use sqlite_storage::SqliteStorage;
use sqlite_storage::semantic_actions::SemanticActionSummary;

use super::action_tree_projection::ActionDisplayProjection;
use crate::json;

const NODE_ID_AGENT: &str = "agent-process";

pub(super) fn action_tree_json(
    storage: &mut SqliteStorage,
    trace_id: TraceId,
) -> Result<String, String> {
    let projection = ActionDisplayProjection::load(storage, trace_id)?;
    let roots = projection
        .root_action_ids
        .iter()
        .map(|action_id| json::string(action_id))
        .collect::<Vec<_>>();
    let action_rows = projection
        .actions
        .iter()
        .map(action_json)
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
    storage: &mut SqliteStorage,
    trace_id: TraceId,
) -> Result<String, String> {
    let summary = storage.semantic_action_summary(trace_id).map_err(|error| {
        format!(
            "read semantic action summary failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let projection = ActionDisplayProjection::load(storage, trace_id)?;
    let observed_agent = storage
        .observed_agent_semantic_action(trace_id)
        .map_err(|error| {
            format!(
                "read observed agent action failed: {}: {}",
                error.stage, error.message
            )
        })?;
    let child_count = projection.root_child_count();

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
    Ok(output)
}

pub(super) fn action_tree_children_json(
    storage: &mut SqliteStorage,
    trace_id: TraceId,
    parent_id: &str,
) -> Result<String, String> {
    let projection = ActionDisplayProjection::load(storage, trace_id)?;
    let rows = if parent_id == NODE_ID_AGENT {
        projection.root_children()
    } else {
        projection.children(parent_id)
    };
    let action_rows = rows
        .iter()
        .map(|row| action_json(&row.action))
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

pub(super) fn action_json(action: &SemanticAction) -> String {
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
    json::field(&mut output, "attributes", &json::map(&action.attributes));
    output.push(',');
    json::field(&mut output, "evidence", &evidence_json(&action.evidence));
    output.push('}');
    output
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
    json::field(&mut output, "evidence", &evidence_json(&link.evidence));
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
