//! OpenTelemetry export from storage-backed viewer commands.

use std::path::{Path, PathBuf};

use config_core::daemon::OperatorConfig;
use model_core::ids::TraceId;
use otel_export::render_otlp_json;

use crate::command::ViewInvocation;

use super::export_file::{reject_active_trace_if_disabled, write_new_file};
use super::source;

pub(super) fn write_otel_export(invocation: &ViewInvocation) -> Result<String, String> {
    if invocation.storage_config_path.is_some() {
        return Err("--storage-config is not supported for export-otel".to_string());
    }
    let config = OperatorConfig::load(&invocation.config_path)?;
    let storage_path = invocation
        .storage_path
        .clone()
        .unwrap_or_else(|| config.storage_path.clone());
    let mut storage = source::open_storage(&storage_path)?;
    let trace_id = source::resolve_trace_id(&storage, invocation.trace_id)?;
    reject_active_trace_if_disabled(
        &storage,
        trace_id,
        config.export_config.allow_active_trace_snapshot,
    )?;

    let snapshot = source::read_snapshot(&mut storage, Some(trace_id))?;
    let actions = source::list_all_semantic_actions(&storage, &snapshot)?;
    let json = render_otlp_json(&snapshot.trace, &actions)
        .map_err(|error| format!("export otel failed: {}: {}", error.stage, error.message))?;
    let output_path = invocation
        .output_path
        .clone()
        .unwrap_or_else(|| default_output_path(&config.export_config.output_directory, trace_id));
    write_new_file(&output_path, json.as_bytes())?;
    Ok(format!(
        "exported {} to {}",
        trace_id,
        output_path.display()
    ))
}

fn default_output_path(output_directory: &Path, trace_id: TraceId) -> PathBuf {
    output_directory.join(format!("{}.otlp.json", trace_id))
}
