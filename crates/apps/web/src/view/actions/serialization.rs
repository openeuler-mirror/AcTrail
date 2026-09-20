use semantic_action::{SemanticAction, SemanticActionLink, SemanticEvidence};

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

pub(in crate::view) fn action_json(action: &SemanticAction) -> String {
    render_action_json(action, false)
}

pub(in crate::view) fn action_json_lite(action: &SemanticAction) -> String {
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
        "{{\"id\":{},\"kind\":{},\"title\":{},\"start_time\":{},\"start_time_unix_nanos\":{},\"end_time\":{},\"end_time_unix_nanos\":{},\"duration\":{},\"process\":{},\"status\":{},\"completeness\":{},\"attributes\":{},\"evidence\":{}}}",
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

pub(super) fn link_json(link: &SemanticActionLink) -> String {
    let evidence = if link.evidence.is_empty() {
        "[]".to_string()
    } else {
        evidence_json(&link.evidence)
    };
    format!(
        "{{\"parent\":{},\"child\":{},\"role\":{},\"origin\":{},\"valid\":{},\"attributes\":{},\"evidence\":{}}}",
        json::string(&link.parent_action_id),
        json::string(&link.child_action_id),
        json::string(link.role.as_str()),
        json::string(link.origin.as_str()),
        json::boolean(link.valid),
        json::map(&link.attributes),
        evidence
    )
}

pub(super) fn link_json_lite(link: &SemanticActionLink) -> String {
    format!(
        "{{\"parent\":{},\"child\":{},\"role\":{},\"origin\":{},\"valid\":{}}}",
        json::string(&link.parent_action_id),
        json::string(&link.child_action_id),
        json::string(link.role.as_str()),
        json::string(link.origin.as_str()),
        json::boolean(link.valid)
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
