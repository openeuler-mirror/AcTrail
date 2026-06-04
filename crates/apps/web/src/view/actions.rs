//! Semantic action tree JSON for the web UI.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionLink, SemanticActionReadStore};
use sqlite_storage::SqliteStorage;

use crate::json;

pub(super) fn action_tree_json(
    storage: &mut SqliteStorage,
    trace_id: TraceId,
) -> Result<String, String> {
    let mut actions = storage.list_semantic_actions(trace_id).map_err(|error| {
        format!(
            "list semantic actions failed: {}: {}",
            error.stage, error.message
        )
    })?;
    actions.sort_by(|left, right| {
        (left.start_time, left.action_id.as_str())
            .cmp(&(right.start_time, right.action_id.as_str()))
    });
    actions.dedup_by(|left, right| left.action_id == right.action_id);
    let links = storage
        .list_semantic_action_links(trace_id)
        .map_err(|error| {
            format!(
                "list semantic action links failed: {}: {}",
                error.stage, error.message
            )
        })?;

    let child_actions = links
        .iter()
        .map(|link| link.child_action_id.as_str())
        .collect::<BTreeSet<_>>();
    let roots = actions
        .iter()
        .filter(|action| !child_actions.contains(action.action_id.as_str()))
        .map(|action| json::string(&action.action_id))
        .collect::<Vec<_>>();
    let action_rows = actions.iter().map(action_json).collect::<Vec<_>>();
    let link_rows = links.iter().map(link_json).collect::<Vec<_>>();

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

fn action_json(action: &SemanticAction) -> String {
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
        "end_time",
        &action
            .end_time
            .map(json::time)
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
