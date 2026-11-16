//! The ActionTree path to one action, in the shape of the LLM navigation.

use model_core::ids::TraceId;
use semantic_action::SemanticActionKind;
use serde_json::json;
use storage_core::{SemanticActionDisplayPathEntry, StorageBackend};

use super::action_tree_roles::{DISPLAY_PARENT_ROLES, NODE_ID_AGENT};
use super::actions::ContextCompactDisplay;

pub fn action_tree_path_json(
    storage: &dyn StorageBackend,
    trace_id: TraceId,
    action_id: &str,
) -> Result<String, String> {
    let roles = display_roles();
    let mut path = display_path(storage, trace_id, &roles, action_id)?;
    if let Some(entries) = path.as_mut() {
        normalize_display_path(storage, trace_id, &roles, entries)?;
    }
    let entries: Vec<_> = path
        .iter()
        .flatten()
        .map(|entry| {
            json!({
                "parent_action_id": entry.parent_action_id.as_deref().unwrap_or(NODE_ID_AGENT),
                "action_id": entry.action_id,
                "offset": entry.offset,
                "kind": entry.kind.as_str(),
            })
        })
        .collect();
    Ok(json!({ "found": path.is_some(), "path": entries }).to_string())
}

/// The LLM call the ActionTree shows an `llm.request` under, or `None` while the request has no
/// call or its Trace's observation data is gone.
pub fn llm_call_of_request(
    storage: &dyn StorageBackend,
    trace_id: TraceId,
    request_action_id: &str,
) -> Result<Option<String>, String> {
    Ok(
        display_path(storage, trace_id, &display_roles(), request_action_id)?
            .into_iter()
            .flatten()
            .rev()
            .find(|entry| entry.kind == SemanticActionKind::LlmCall)
            .map(|entry| entry.action_id),
    )
}

fn display_roles() -> Vec<&'static str> {
    DISPLAY_PARENT_ROLES
        .iter()
        .map(|role| role.as_str())
        .collect()
}

fn display_path(
    storage: &dyn StorageBackend,
    trace_id: TraceId,
    roles: &[&str],
    action_id: &str,
) -> Result<Option<Vec<SemanticActionDisplayPathEntry>>, String> {
    storage
        .semantic_action_display_path_to_action(
            trace_id,
            roles,
            &ContextCompactDisplay::navigation_parent_roles(roles),
            action_id,
        )
        .map_err(|error| {
            format!(
                "locate action tree path: {}: {}",
                error.stage, error.message
            )
        })
}

/// Root Compact rows occupy the slot displayed as their implementation call.
pub(in crate::view) fn normalize_display_path(
    storage: &dyn StorageBackend,
    trace_id: TraceId,
    display_roles: &[&str],
    path: &mut [SemanticActionDisplayPathEntry],
) -> Result<(), String> {
    let compact_display = ContextCompactDisplay::for_implementation_ids(
        storage,
        trace_id,
        path.iter()
            .filter(|entry| entry.kind == SemanticActionKind::LlmCall)
            .map(|entry| entry.action_id.as_str()),
    )?;
    for entry in path {
        if entry.parent_action_id.is_none()
            && let Some(compact) = compact_display.compact_for_implementation(&entry.action_id)
        {
            entry.offset = storage
                .semantic_action_display_root_offset(trace_id, display_roles, &compact.action_id)
                .map_err(|error| {
                    format!(
                        "locate implementation root: {}: {}",
                        error.stage, error.message
                    )
                })?
                .ok_or_else(|| "implementation root is absent from the root page".to_string())?;
        }
    }
    Ok(())
}
