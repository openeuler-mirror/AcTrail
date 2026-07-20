//! Storage-backed API view rendering.

#[path = "view/action_tree_projection.rs"]
mod action_tree_projection;
#[path = "view/action_tree_roles.rs"]
mod action_tree_roles;
#[path = "view/actions.rs"]
mod actions;
#[path = "view/cluster.rs"]
pub(crate) mod cluster;
#[path = "view/commands.rs"]
mod commands;
#[path = "view/events.rs"]
mod events;
#[path = "view/payloads.rs"]
mod payloads;
#[path = "view/projection_cache.rs"]
mod projection_cache;
#[path = "view/runtime_config.rs"]
mod runtime_config;
#[path = "view/stats.rs"]
mod stats;
#[path = "view/topology.rs"]
mod topology;
#[path = "view/traces.rs"]
mod traces;

use model_core::ids::TraceId;
use model_core::payload::PayloadSegmentId;
use storage_core::{
    PayloadSegmentQuery, SemanticActionChildPageQuery, SnapshotView, StorageBackend,
    StorageOpenMode, TraceFilter,
};
use storage_factory::{StorageConfig, open_storage_backend};

use crate::json;

pub(crate) use stats::{
    ExportView, LlmActivityQuery, LlmExportQuery, LlmRowsQuery, Rollup, TokenUsageStatsQuery,
};

pub fn current_config_json(
    config_path: Option<&std::path::Path>,
    operator_config: Option<&config_core::daemon::OperatorConfig>,
) -> Result<String, String> {
    runtime_config::current_config_json(config_path, operator_config)
}

pub fn plugin_enablement_json(
    config_path: Option<&std::path::Path>,
    operator_config: Option<&config_core::daemon::OperatorConfig>,
) -> Result<String, String> {
    runtime_config::plugin_enablement_json(config_path, operator_config)
}

pub fn runtime_plugin_status_json(
    config_path: Option<&std::path::Path>,
    operator_config: Option<&config_core::daemon::OperatorConfig>,
) -> Result<String, String> {
    runtime_config::runtime_plugin_status_json(config_path, operator_config)
}

pub fn runtime_plugin_unload_json(
    config_path: Option<&std::path::Path>,
    operator_config: Option<&config_core::daemon::OperatorConfig>,
    instance_id: &str,
) -> Result<String, String> {
    runtime_config::runtime_plugin_unload_json(config_path, operator_config, instance_id)
}

pub fn traces_json(storage_config: &StorageConfig) -> Result<String, String> {
    let storage = open_storage(storage_config)?;
    let process_display = traces::ProcessDisplayIndex::load(storage.as_ref())?;
    let traces = storage
        .list_traces(&TraceFilter::default())
        .map_err(|error| format!("list traces failed: {}: {}", error.stage, error.message))?;
    let rows = traces
        .iter()
        .map(|trace| process_display.render_trace(trace))
        .collect::<Vec<_>>();
    Ok(format!("{{\"traces\":[{}]}}", rows.join(",")))
}

pub fn token_usage_stats_json(
    storage_config: &StorageConfig,
    query: stats::TokenUsageStatsQuery,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    stats::token_usage_stats_json(storage.as_mut(), query)
}

pub fn llm_activity_json(
    storage_config: &StorageConfig,
    query: stats::LlmActivityQuery,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    stats::llm_activity_json(storage.as_mut(), query)
}

pub fn llm_request_rows_json(
    storage_config: &StorageConfig,
    query: stats::LlmRowsQuery,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    stats::llm_request_rows_json(storage.as_mut(), query)
}

pub fn llm_explore_json(
    storage_config: &StorageConfig,
    query: stats::LlmExploreQuery,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    stats::llm_explore_json(storage.as_mut(), query)
}

pub fn llm_export_csv(
    storage_config: &StorageConfig,
    query: stats::LlmExportQuery,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    stats::llm_export_csv(storage.as_mut(), query)
}

pub fn parse_llm_explore_query(body: &str) -> Result<stats::LlmExploreQuery, String> {
    stats::parse_explore_query(body)
}

pub fn trace_json(storage_config: &StorageConfig, trace_id: u64) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    let process_display = traces::ProcessDisplayIndex::load(storage.as_ref())?;
    let trace_id = TraceId::new(trace_id);
    let snapshot = read_snapshot(storage.as_mut(), trace_id)?;
    let payloads = storage
        .list_payload_segments(trace_id, PayloadSegmentQuery::metadata_only(None))
        .map_err(|error| format!("list payloads failed: {}: {}", error.stage, error.message))?;
    let retained_payload_bytes = storage.retained_payload_bytes(trace_id).map_err(|error| {
        format!(
            "read retained payload bytes failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let counts = events::event_counts(&snapshot.events);
    let events = snapshot
        .events
        .iter()
        .map(events::event_json)
        .collect::<Vec<_>>();
    let processes = snapshot
        .memberships
        .iter()
        .map(events::process_json)
        .collect::<Vec<_>>();
    let diagnostics = snapshot
        .diagnostics
        .iter()
        .map(events::diagnostic_json)
        .collect::<Vec<_>>();
    let payload_rows = payloads
        .iter()
        .map(payloads::payload_json_row)
        .collect::<Vec<_>>();
    let process_tree = topology::process_tree_json(&snapshot.memberships, &process_display);
    let timeline = topology::timeline_json(&snapshot.events, &payloads);

    let mut output = String::from("{");
    json::field(
        &mut output,
        "trace",
        &process_display.render_trace(&snapshot.trace),
    );
    output.push(',');
    json::field(
        &mut output,
        "counts",
        &events::counts_json(&counts, retained_payload_bytes, payloads.len()),
    );
    output.push(',');
    json::field(&mut output, "events", &format!("[{}]", events.join(",")));
    output.push(',');
    json::field(
        &mut output,
        "processes",
        &format!("[{}]", processes.join(",")),
    );
    output.push(',');
    json::field(&mut output, "process_tree", &process_tree);
    output.push(',');
    json::field(
        &mut output,
        "payloads",
        &format!("[{}]", payload_rows.join(",")),
    );
    output.push(',');
    json::field(&mut output, "timeline", &timeline);
    output.push(',');
    json::field(
        &mut output,
        "diagnostics",
        &format!("[{}]", diagnostics.join(",")),
    );
    output.push('}');
    Ok(output)
}

pub fn trace_summary_json(storage_config: &StorageConfig, trace_id: u64) -> Result<String, String> {
    let storage = open_storage(storage_config)?;
    let process_display = traces::ProcessDisplayIndex::load(storage.as_ref())?;
    let trace_id = TraceId::new(trace_id);
    let trace = storage
        .get_trace(trace_id)
        .map_err(|error| format!("read trace failed: {}: {}", error.stage, error.message))?
        .ok_or_else(|| format!("trace {trace_id} not found"))?;
    let variant_counts = storage.count_events_by_variant(trace_id).map_err(|error| {
        format!(
            "count events by variant failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let payload_segments = storage.count_payload_segments(trace_id).map_err(|error| {
        format!(
            "count payload segments failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let retained_payload_bytes = storage.retained_payload_bytes(trace_id).map_err(|error| {
        format!(
            "read retained payload bytes failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let counts = events::event_counts_from_variants(&variant_counts);

    let mut output = String::from("{");
    json::field(&mut output, "trace", &process_display.render_trace(&trace));
    output.push(',');
    json::field(
        &mut output,
        "counts",
        &events::counts_json(&counts, retained_payload_bytes, payload_segments),
    );
    output.push('}');
    Ok(output)
}

pub fn trace_events_json(storage_config: &StorageConfig, trace_id: u64) -> Result<String, String> {
    let storage = open_storage(storage_config)?;
    let trace_id = TraceId::new(trace_id);
    let events = storage
        .list_events(trace_id)
        .map_err(|error| format!("list events failed: {}: {}", error.stage, error.message))?;
    let rows = events.iter().map(events::event_json).collect::<Vec<_>>();
    Ok(format!("{{\"events\":[{}]}}", rows.join(",")))
}

pub fn trace_payloads_json(
    storage_config: &StorageConfig,
    trace_id: u64,
) -> Result<String, String> {
    let storage = open_storage(storage_config)?;
    let payloads = storage
        .list_payload_segments(
            TraceId::new(trace_id),
            PayloadSegmentQuery::metadata_only(None),
        )
        .map_err(|error| format!("list payloads failed: {}: {}", error.stage, error.message))?;
    let rows = payloads
        .iter()
        .map(payloads::payload_json_row)
        .collect::<Vec<_>>();
    Ok(format!("{{\"payloads\":[{}]}}", rows.join(",")))
}

pub fn trace_timeline_json(
    storage_config: &StorageConfig,
    trace_id: u64,
) -> Result<String, String> {
    let storage = open_storage(storage_config)?;
    let trace_id = TraceId::new(trace_id);
    let events = storage
        .list_events(trace_id)
        .map_err(|error| format!("list events failed: {}: {}", error.stage, error.message))?;
    let payloads = storage
        .list_payload_segments(trace_id, PayloadSegmentQuery::metadata_only(None))
        .map_err(|error| format!("list payloads failed: {}: {}", error.stage, error.message))?;
    Ok(format!(
        "{{\"timeline\":{}}}",
        topology::timeline_json(&events, &payloads)
    ))
}

pub fn trace_processes_json(
    storage_config: &StorageConfig,
    trace_id: u64,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    let process_display = traces::ProcessDisplayIndex::load(storage.as_ref())?;
    let snapshot = read_snapshot(storage.as_mut(), TraceId::new(trace_id))?;
    let processes = snapshot
        .memberships
        .iter()
        .map(events::process_json)
        .collect::<Vec<_>>();
    let process_tree = topology::process_tree_json(&snapshot.memberships, &process_display);
    Ok(format!(
        "{{\"processes\":[{}],\"process_tree\":{}}}",
        processes.join(","),
        process_tree
    ))
}

pub fn trace_diagnostics_json(
    storage_config: &StorageConfig,
    trace_id: u64,
) -> Result<String, String> {
    let storage = open_storage(storage_config)?;
    let diagnostics = storage
        .list_diagnostics(TraceId::new(trace_id))
        .map_err(|error| {
            format!(
                "list diagnostics failed: {}: {}",
                error.stage, error.message
            )
        })?;
    let rows = diagnostics
        .iter()
        .map(events::diagnostic_json)
        .collect::<Vec<_>>();
    Ok(format!("{{\"diagnostics\":[{}]}}", rows.join(",")))
}

pub fn clear_cache_json() -> Result<String, String> {
    Ok(projection_cache::clear_projection_cache_json())
}

pub fn action_tree_json(storage_config: &StorageConfig, trace_id: u64) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    actions::action_tree_json(
        storage_config.path(),
        storage.as_mut(),
        TraceId::new(trace_id),
    )
}

pub fn action_tree_root_json(
    storage_config: &StorageConfig,
    trace_id: u64,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    actions::action_tree_root_json(
        storage_config.path(),
        storage.as_mut(),
        TraceId::new(trace_id),
    )
}

pub fn action_tree_children_json(
    storage_config: &StorageConfig,
    trace_id: u64,
    parent_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    actions::action_tree_children_json(
        storage_config.path(),
        storage.as_mut(),
        TraceId::new(trace_id),
        parent_id,
        page,
    )
}

pub fn action_detail_json(
    storage_config: &StorageConfig,
    trace_id: u64,
    action_id: &str,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    actions::action_detail_json(storage.as_mut(), TraceId::new(trace_id), action_id)
}

pub fn action_file_path_set_json(
    storage_config: &StorageConfig,
    trace_id: u64,
    action_id: &str,
    page: SemanticActionChildPageQuery,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    actions::file_path_set_json(storage.as_mut(), TraceId::new(trace_id), action_id, page)
}

pub fn action_llm_request_content_json(
    storage_config: &StorageConfig,
    trace_id: u64,
    action_id: &str,
    max_bytes: usize,
) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    actions::llm_request_content_json(
        storage.as_mut(),
        TraceId::new(trace_id),
        action_id,
        max_bytes,
    )
}

pub fn commands_json(storage_config: &StorageConfig, trace_id: u64) -> Result<String, String> {
    let mut storage = open_storage(storage_config)?;
    commands::commands_json(storage.as_mut(), TraceId::new(trace_id))
}

pub fn payload_json(
    storage_config: &StorageConfig,
    trace_id: u64,
    segment_id: u64,
) -> Result<String, String> {
    let storage = open_storage(storage_config)?;
    let mut segments = storage
        .list_payload_segments(
            TraceId::new(trace_id),
            PayloadSegmentQuery {
                segment_id: Some(PayloadSegmentId::new(segment_id)),
                direction: None,
                limit: None,
                include_bytes: true,
            },
        )
        .map_err(|error| format!("list payload failed: {}: {}", error.stage, error.message))?;
    let segment = segments
        .pop()
        .ok_or_else(|| format!("payload segment {segment_id} not found"))?;
    Ok(payloads::payload_json_with_bytes(&segment))
}

fn open_storage(storage_config: &StorageConfig) -> Result<Box<dyn StorageBackend>, String> {
    open_storage_backend(storage_config, StorageOpenMode::ReadOnly).map_err(|error| {
        format!(
            "open storage read-only failed: {}: {}",
            error.stage, error.message
        )
    })
}

fn read_snapshot(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<SnapshotView, String> {
    let lease = storage.acquire_export_lease(trace_id).map_err(|error| {
        format!(
            "acquire snapshot lease failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let snapshot = storage
        .read_snapshot(&lease)
        .map_err(|error| format!("read snapshot failed: {}: {}", error.stage, error.message));
    let release = storage.release_export_lease(lease).map_err(|error| {
        format!(
            "release snapshot lease failed: {}: {}",
            error.stage, error.message
        )
    });
    match (snapshot, release) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}
