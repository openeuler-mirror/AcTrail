//! Storage access for viewer commands.

use config_core::daemon::OperatorConfig;
use model_core::ids::TraceId;
use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionLink};
use storage_core::{
    PayloadSegmentQuery, SnapshotView, StorageBackend, StorageOpenMode, TraceFilter,
};
use storage_factory::{StorageConfig, open_storage_backend};

use crate::command::ViewInvocation;

pub(super) fn storage_config(invocation: &ViewInvocation) -> Result<StorageConfig, String> {
    if invocation.storage_path.is_some() && invocation.storage_config_path.is_some() {
        return Err("--storage-path cannot be combined with --storage-config".to_string());
    }
    if let Some(path) = &invocation.storage_path {
        return Ok(StorageConfig::sqlite_path(path));
    }
    if let Some(path) = &invocation.storage_config_path {
        return OperatorConfig::load(path).map(|config| config.storage);
    }
    OperatorConfig::load(&invocation.config_path).map(|config| config.storage)
}

pub(super) fn open_storage(config: &StorageConfig) -> Result<Box<dyn StorageBackend>, String> {
    if !config.path().exists() {
        return Err(format!(
            "storage path does not exist: {}",
            config.path().display()
        ));
    }
    open_storage_backend(config, StorageOpenMode::ReadOnly)
        .map_err(|error| format!("open storage {}: {}", error.stage, error.message))
}

pub(super) fn list_traces(storage: &dyn StorageBackend) -> Result<Vec<TraceRecord>, String> {
    storage
        .list_traces(&TraceFilter::default())
        .map_err(|error| format!("list traces failed: {}: {}", error.stage, error.message))
}

pub(super) fn list_payload_segments(
    storage: &dyn StorageBackend,
    trace_id: model_core::ids::TraceId,
    query: PayloadSegmentQuery,
) -> Result<Vec<model_core::payload::PayloadSegment>, String> {
    storage
        .list_payload_segments(trace_id, query)
        .map_err(|error| format!("{}: {}", error.stage, error.message))
}

pub(super) fn list_semantic_actions(
    storage: &dyn StorageBackend,
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
    storage: &dyn StorageBackend,
    trace_id: TraceId,
) -> Result<Vec<SemanticActionLink>, String> {
    storage
        .list_semantic_action_links(trace_id)
        .map_err(|error| format!("{}: {}", error.stage, error.message))
}

pub(super) fn read_snapshot(
    storage: &mut dyn StorageBackend,
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
    storage: &dyn StorageBackend,
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
