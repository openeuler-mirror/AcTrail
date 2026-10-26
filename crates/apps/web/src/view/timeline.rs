//! Bounded semantic timeline and inexpensive trace metadata.

use model_core::ids::TraceId;
use storage_core::TimelineQuery;
use storage_factory::StorageConfig;

use super::{actions, open_storage, traces::ProcessDisplayIndex};
use crate::json;

pub(crate) fn trace_metadata_json(config: &StorageConfig, trace_id: u64) -> Result<String, String> {
    let storage = open_storage(config)?;
    let trace = storage
        .get_trace(TraceId::new(trace_id))
        .map_err(|error| format!("read trace: {}: {}", error.stage, error.message))?
        .ok_or_else(|| format!("trace {trace_id} not found"))?;
    let display = ProcessDisplayIndex::for_trace(storage.as_ref(), &trace)?;
    let bounds = storage
        .timeline_bounds(trace.trace_id)
        .map_err(|error| format!("read timeline bounds: {}: {}", error.stage, error.message))?;
    Ok(format!(
        "{{\"trace\":{},\"timeline_bounds\":{{\"start_time_unix_nanos\":{},\"end_time_unix_nanos\":{}}}}}",
        display.render_trace(&trace),
        json::optional_time_nanos(bounds.first_start_time),
        json::optional_time_nanos(bounds.last_start_time),
    ))
}

pub(crate) struct TimelineView;

impl TimelineView {
    pub(crate) fn render(config: &StorageConfig, query: &TimelineQuery) -> Result<String, String> {
        let storage = open_storage(config)?;
        let page = storage
            .timeline_page(query)
            .map_err(|error| format!("read timeline: {}: {}", error.stage, error.message))?;
        let actions = page
            .actions
            .iter()
            .map(actions::action_json_lite)
            .collect::<Vec<_>>();
        let context = page
            .context_actions
            .iter()
            .map(actions::action_json_lite)
            .collect::<Vec<_>>();
        let links = page
            .links
            .iter()
            .map(|link| {
                format!(
                    "{{\"parent\":{},\"child\":{},\"role\":{},\"origin\":{},\"valid\":{}}}",
                    json::string(&link.parent_action_id),
                    json::string(&link.child_action_id),
                    json::string(link.role.as_str()),
                    json::string(link.origin.as_str()),
                    link.valid
                )
            })
            .collect::<Vec<_>>();
        let cursor = page
            .next_cursor
            .as_ref()
            .map(|cursor| {
                let nanos = cursor
                    .start_time
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| "timeline cursor precedes epoch".to_string())?
                    .as_nanos();
                Ok::<_, String>(json::string(&format!("{nanos}:{}", cursor.action_key)))
            })
            .transpose()?
            .unwrap_or_else(|| "null".into());
        Ok(format!(
            "{{\"actions\":[{}],\"context_actions\":[{}],\"links\":[{}],\"next_cursor\":{},\"has_more\":{},\"scanned\":{},\"links_truncated\":{},\"partial\":true}}",
            actions.join(","),
            context.join(","),
            links.join(","),
            cursor,
            page.next_cursor.is_some(),
            page.scanned,
            page.links_truncated,
        ))
    }
}

/// Counts are loaded separately so index walks never delay the initial viewport.
pub(crate) fn trace_counts_json(config: &StorageConfig, trace_id: u64) -> Result<String, String> {
    let storage = open_storage(config)?;
    let counts = storage
        .trace_counts(TraceId::new(trace_id))
        .map_err(|error| format!("{}: {}", error.stage, error.message))?;
    Ok(
        serde_json::json!({"counts": {"events": counts.events, "payloads": counts.payloads,
        "actions": counts.actions, "processes": counts.processes}})
        .to_string(),
    )
}
