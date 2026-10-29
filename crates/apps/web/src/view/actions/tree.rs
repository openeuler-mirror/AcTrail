use std::path::Path;

use model_core::ids::TraceId;
use semantic_action::SemanticActionKind;
use storage_core::{SemanticActionChildPageQuery, StorageBackend};

use super::super::LlmNavMode;
use super::super::action_tree_roles::{NODE_ID_AGENT, ROOT_LINK_ROLES};
use super::serialization::link_json;
use super::{
    ContextCompactDisplay, DisplayChild, LlmCallDisplay, LlmToolDisplay, bool_json,
    child_state_json, display_parent_role_names, has_more_children, json, next_offset_json,
    storage_error,
};

pub(in crate::view) fn action_tree_children_json(
    _storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let display_roles = display_parent_role_names();
    let parent = if parent_id == NODE_ID_AGENT {
        None
    } else {
        storage
            .semantic_actions_by_ids_lite(trace_id, &[parent_id])
            .map_err(|error| storage_error("read tree parent", error))?
            .into_iter()
            .next()
    };
    let (mut rows, mut total) = if parent
        .as_ref()
        .is_some_and(|parent| parent.kind == SemanticActionKind::AgentContextCompact)
    {
        (Vec::new(), 0)
    } else if parent_id == NODE_ID_AGENT {
        let root_roles = ROOT_LINK_ROLES
            .iter()
            .map(|role| role.as_str())
            .collect::<Vec<_>>();
        let result = storage
            .semantic_action_display_root_children_page(
                trace_id,
                &display_roles,
                &root_roles,
                &ContextCompactDisplay::excluded_fallback_kinds(),
                page,
            )
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
    let compact_display = ContextCompactDisplay::for_actions(
        storage,
        trace_id,
        parent.iter().chain(rows.iter().map(|row| &row.action)),
    )?;
    if let Some(marker) = compact_display.compact_for_implementation(parent_id) {
        if page.offset <= total && total < page.offset.saturating_add(page.limit) {
            rows.push(DisplayChild {
                action: marker.clone(),
                link: None,
                child_count: 0,
            });
        }
        total = total.saturating_add(1);
    }
    if parent_id == NODE_ID_AGENT {
        compact_display.project_root_rows(storage, trace_id, &mut rows, &display_roles)?;
    }
    for row in &mut rows {
        if row.action.kind == SemanticActionKind::AgentContextCompact {
            row.child_count = 0;
        } else if compact_display
            .compact_for_implementation(&row.action.action_id)
            .is_some()
        {
            row.child_count = row.child_count.saturating_add(1);
        }
        LlmCallDisplay::normalize_one(storage, &mut row.action)?;
        LlmToolDisplay::normalize_one(storage, &mut row.action);
    }
    let actions = rows
        .iter()
        .map(|row| compact_display.action_json(&row.action, true))
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

pub(in crate::view) fn action_tree_llm_nav_json(
    _storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
    mode: LlmNavMode,
    after_action_id: Option<&str>,
) -> Result<String, String> {
    let display_roles = display_parent_role_names();
    let after = match mode {
        LlmNavMode::First => None,
        LlmNavMode::Next => {
            if let Some(id) = after_action_id.filter(|id| id.ends_with(":agent.context_compaction"))
            {
                let action = storage
                    .semantic_action_by_id(trace_id, id)
                    .map_err(|error| storage_error("read navigation anchor", error))?;
                let compact = ContextCompactDisplay::for_actions(storage, trace_id, action.iter())?;
                Some(
                    compact
                        .implementation_calls()
                        .next()
                        .map(|call| call.action_id.clone())
                        .ok_or_else(|| {
                            "compact navigation anchor has no valid implementation".to_string()
                        })?,
                )
            } else {
                after_action_id.map(str::to_string)
            }
        }
    };
    let path = storage
        .semantic_action_display_path_to_kind(
            trace_id,
            &display_roles,
            &ContextCompactDisplay::navigation_parent_roles(&display_roles),
            "llm.call",
            after.as_deref(),
        )
        .map_err(|error| storage_error("navigate action tree LLM calls", error))?;
    let mut path = path;
    let compact_display = ContextCompactDisplay::for_implementation_ids(
        storage,
        trace_id,
        path.iter()
            .flatten()
            .filter(|entry| entry.kind == SemanticActionKind::LlmCall)
            .map(|entry| entry.action_id.as_str()),
    )?;
    if let Some(entries) = path.as_mut() {
        for entry in entries {
            if entry.parent_action_id.is_none()
                && let Some(compact) = compact_display.compact_for_implementation(&entry.action_id)
            {
                entry.offset = storage
                    .semantic_action_display_root_offset(
                        trace_id,
                        &display_roles,
                        &compact.action_id,
                    )
                    .map_err(|error| storage_error("locate implementation root", error))?
                    .ok_or_else(|| {
                        "implementation root is absent from the root page".to_string()
                    })?;
            }
        }
    }
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
