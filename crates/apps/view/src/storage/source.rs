//! Storage access for viewer commands.

use std::path::{Path, PathBuf};

use config_core::daemon::OperatorConfig;
use model_core::ids::TraceId;
use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionLink, SemanticActionReadStore};
use sqlite_storage::SqliteStorage;
use store_read_contract::filters::TraceFilter;
use store_read_contract::payloads::{PayloadReadStore, PayloadSegmentQuery};
use store_read_contract::traces::TraceReadStore;
use store_snapshot_contract::lease::SnapshotLeaseStore;
use store_snapshot_contract::view::{SnapshotStore, SnapshotView};

use crate::command::ViewInvocation;

pub(super) fn storage_path(invocation: &ViewInvocation) -> Result<PathBuf, String> {
    if invocation.storage_path.is_some() && invocation.storage_config_path.is_some() {
        return Err("--storage-path cannot be combined with --storage-config".to_string());
    }
    if let Some(path) = &invocation.storage_path {
        return Ok(path.clone());
    }
    if let Some(path) = &invocation.storage_config_path {
        return storage_path_from_config(path);
    }
    OperatorConfig::load(&invocation.config_path).map(|config| config.storage_path)
}

fn storage_path_from_config(path: &Path) -> Result<PathBuf, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    for (line_index, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (key, value) = trimmed
            .split_once('=')
            .ok_or_else(|| format!("invalid config line {}", line_index + 1))?;
        if key.trim() == "storage_path" {
            return Ok(PathBuf::from(unquote(value.trim())?));
        }
    }
    Err(format!(
        "missing config key storage_path in {}",
        path.display()
    ))
}

fn unquote(value: &str) -> Result<String, String> {
    if value.starts_with('"') || value.ends_with('"') {
        if !(value.starts_with('"') && value.ends_with('"') && value.len() >= 2) {
            return Err(format!("invalid quoted value {value}"));
        }
        return Ok(value[1..value.len() - 1].to_string());
    }
    Ok(value.to_string())
}

pub(super) fn open_storage(path: &PathBuf) -> Result<SqliteStorage, String> {
    if !path.exists() {
        return Err(format!("storage path does not exist: {}", path.display()));
    }
    SqliteStorage::open_read_only(path)
        .map_err(|error| format!("open storage {}: {error}", path.display()))
}

pub(super) fn list_traces(storage: &SqliteStorage) -> Result<Vec<TraceRecord>, String> {
    storage
        .list_traces(&TraceFilter::default())
        .map_err(|error| format!("list traces failed: {}: {}", error.stage, error.message))
}

pub(super) fn list_payload_segments(
    storage: &SqliteStorage,
    trace_id: model_core::ids::TraceId,
    query: PayloadSegmentQuery,
) -> Result<Vec<model_core::payload::PayloadSegment>, String> {
    storage
        .list_payload_segments(trace_id, query)
        .map_err(|error| format!("{}: {}", error.stage, error.message))
}

pub(super) fn list_semantic_actions(
    storage: &SqliteStorage,
    trace_id: TraceId,
) -> Result<Vec<SemanticAction>, String> {
    let mut actions = storage
        .list_semantic_actions(trace_id)
        .map_err(|error| format!("{}: {}", error.stage, error.message))?;
    actions.sort_by(|left, right| {
        (left.start_time, left.action_id.as_str())
            .cmp(&(right.start_time, right.action_id.as_str()))
    });
    actions.dedup_by(|left, right| left.action_id == right.action_id);
    Ok(actions)
}

pub(super) fn list_semantic_action_links(
    storage: &SqliteStorage,
    trace_id: TraceId,
) -> Result<Vec<SemanticActionLink>, String> {
    storage
        .list_semantic_action_links(trace_id)
        .map_err(|error| format!("{}: {}", error.stage, error.message))
}

pub(super) fn read_snapshot(
    storage: &mut SqliteStorage,
    requested: Option<TraceId>,
) -> Result<SnapshotView, String> {
    let trace_id = resolve_trace_id(storage, requested)?;
    let lease = storage.acquire_export_lease(trace_id).map_err(|error| {
        format!(
            "acquire snapshot lease failed: {}: {}",
            error.stage, error.message
        )
    })?;
    let snapshot = storage.read_snapshot(&lease);
    let release = storage.release_export_lease(lease);
    match (snapshot, release) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Err(error), Ok(())) => Err(format!(
            "read snapshot failed: {}: {}",
            error.stage, error.message
        )),
        (Ok(_), Err(error)) => Err(format!(
            "release snapshot lease failed: {}: {}",
            error.stage, error.message
        )),
        (Err(read_error), Err(release_error)) => Err(format!(
            "read snapshot failed: {}: {}; release snapshot lease failed: {}: {}",
            read_error.stage, read_error.message, release_error.stage, release_error.message
        )),
    }
}

pub(super) fn resolve_trace_id(
    storage: &SqliteStorage,
    requested: Option<TraceId>,
) -> Result<TraceId, String> {
    if let Some(trace_id) = requested {
        return Ok(trace_id);
    }
    let traces = list_traces(storage)?;
    match traces.as_slice() {
        [] => Err("no traces found".to_string()),
        [trace] => Ok(trace.trace_id),
        _ => {
            Err("multiple traces found; run `actrailviewer traces` and pass --trace-id".to_string())
        }
    }
}
