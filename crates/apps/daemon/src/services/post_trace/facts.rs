use std::path::{Path, PathBuf};
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::trace::{TraceLifecycleState, TraceRecord};
use plugin_system::{
    PluginRuntimeError, TraceAnalysisAction, TraceAnalysisContext, TraceAnalysisFileChange,
    TraceFileState, TraceFileStateStatus,
};
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStatus, attr_keys};
use storage_core::StorageBackend;

pub(super) fn analysis_context(
    trace: &TraceRecord,
) -> Result<TraceAnalysisContext, PluginRuntimeError> {
    let terminal_at = match trace.lifecycle_state {
        TraceLifecycleState::Completed => trace.timings.completed_at,
        TraceLifecycleState::Exited => trace.timings.exited_at,
        TraceLifecycleState::Failed => trace.timings.failed_at,
        _ => None,
    }
    .ok_or_else(|| {
        PluginRuntimeError::new(
            "trace_analysis",
            format!("trace {} is not durably terminal", trace.trace_id),
        )
    })?;
    Ok(TraceAnalysisContext {
        terminal_at,
        working_directory: trace.root_working_directory.clone(),
    })
}

pub(super) fn project_analysis_action(
    action: SemanticAction,
    paths: Vec<semantic_action::FileObservationPath>,
) -> TraceAnalysisAction {
    let path_complete = paths.len() == 1;
    let path = path_complete.then(|| paths[0].path.clone());
    let file_change = matches!(
        action.kind,
        SemanticActionKind::FileModify | SemanticActionKind::FileWrite
    )
    .then(|| TraceAnalysisFileChange {
        operation: action
            .attributes
            .get(attr_keys::file::OPERATION)
            .cloned()
            .unwrap_or_else(|| match action.kind {
                SemanticActionKind::FileWrite => "write".to_string(),
                _ => action.kind.as_str().to_string(),
            }),
        change_kind: action.file_change_kind().unwrap_or(match action.kind {
            SemanticActionKind::FileWrite => semantic_action::FileChangeKind::Modified,
            _ => semantic_action::FileChangeKind::Unknown,
        }),
        successful: action.status == SemanticActionStatus::Success,
        path: path.or_else(|| action.attributes.get(attr_keys::file::PATH).cloned()),
        path_complete: path_complete || action.attributes.contains_key(attr_keys::file::PATH),
    });
    TraceAnalysisAction {
        action_id: action.action_id,
        kind: action.kind.as_str().to_string(),
        status: action.status,
        completeness: action.completeness,
        file_change,
    }
}

pub(super) fn observed_host_path(
    storage: &dyn StorageBackend,
    trace: &TraceRecord,
    action: &SemanticAction,
    observed_path: &str,
) -> Result<Option<PathBuf>, PluginRuntimeError> {
    let observed_path = PathBuf::from(observed_path);
    let absolute = if observed_path.is_absolute() {
        observed_path
    } else {
        let Some(working_directory) = trace.root_working_directory.as_deref() else {
            return Ok(None);
        };
        PathBuf::from(working_directory).join(observed_path)
    };
    if trace.root_container_id.is_none() {
        return Ok(Some(absolute));
    }
    let process = storage
        .get_process_record(action.process.clone())
        .map_err(storage_runtime_error)?
        .ok_or_else(|| {
            PluginRuntimeError::new(
                "trace_file_state",
                format!("process {} is missing from storage", action.process),
            )
        })?;
    let Some(namespace) = process.namespaces.iter().next() else {
        return Ok(None);
    };
    Ok(ebpf_collector::procfs::resolve_host_path(
        &namespace.pid_namespace,
        &absolute.to_string_lossy(),
    ))
}

pub(super) fn read_file_state(path: &Path) -> TraceFileState {
    let checked_at = SystemTime::now();
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => TraceFileState {
            status: TraceFileStateStatus::Exists,
            checked_at,
            file_kind: Some(
                if metadata.file_type().is_file() {
                    "file"
                } else if metadata.file_type().is_dir() {
                    "directory"
                } else if metadata.file_type().is_symlink() {
                    "symlink"
                } else {
                    "other"
                }
                .to_string(),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => TraceFileState {
            status: TraceFileStateStatus::NotFound,
            checked_at,
            file_kind: None,
        },
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => TraceFileState {
            status: TraceFileStateStatus::Inaccessible,
            checked_at,
            file_kind: None,
        },
        Err(_) => TraceFileState {
            status: TraceFileStateStatus::Unavailable,
            checked_at,
            file_kind: None,
        },
    }
}

pub(super) fn trace_missing(trace_id: TraceId) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "trace_analysis",
        format!("trace {trace_id} was not found in durable storage"),
    )
}

pub(super) fn storage_runtime_error(error: storage_core::StorageError) -> PluginRuntimeError {
    PluginRuntimeError::new(error.stage, error.message)
}
