//! Shared file-export helpers.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use model_core::ids::TraceId;
use model_core::trace::TraceLifecycleState;
use storage_core::StorageBackend;

pub(super) fn reject_active_trace_if_disabled(
    storage: &dyn StorageBackend,
    trace_id: TraceId,
    allow_active: bool,
) -> Result<(), String> {
    if allow_active {
        return Ok(());
    }
    let trace = storage
        .get_trace(trace_id)
        .map_err(|error| format!("read trace failed: {}: {}", error.stage, error.message))?
        .ok_or_else(|| "trace not found in storage".to_string())?;
    if matches!(
        trace.lifecycle_state,
        TraceLifecycleState::Starting | TraceLifecycleState::Active | TraceLifecycleState::Draining
    ) {
        return Err("active trace export is disabled by configuration".to_string());
    }
    Ok(())
}

pub(super) fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create export directory {}: {error}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create export file {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("write export file {}: {error}", path.display()))
}
