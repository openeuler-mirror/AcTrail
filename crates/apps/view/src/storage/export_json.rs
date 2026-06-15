//! JSON graph export from storage-backed viewer commands.

use std::path::{Path, PathBuf};

use config_core::daemon::OperatorConfig;
use json_graph_export::service::JsonGraphExportService;
use model_core::ids::TraceId;

use crate::command::ViewInvocation;

use super::export_file::{reject_active_trace_if_disabled, write_new_file};
use super::source;

pub(super) fn write_json_export(invocation: &ViewInvocation) -> Result<String, String> {
    let config = OperatorConfig::load(&invocation.config_path)?;
    let storage_config = source::storage_config(invocation)?;
    let mut storage = source::open_storage(&storage_config)?;
    let trace_id = source::resolve_trace_id(storage.as_ref(), invocation.trace_id)?;
    reject_active_trace_if_disabled(
        storage.as_ref(),
        trace_id,
        config.export_config.allow_active_trace_snapshot,
    )?;

    let mut exporter = JsonGraphExportService::new(
        storage.as_mut(),
        config.export_config.graph_schema_version,
        config.export_config.payload_bytes_enabled,
        config.export_config.payload_text_enabled,
    );
    let json = exporter
        .export_json(trace_id)
        .map_err(|error| format!("export json failed: {}: {}", error.stage, error.message))?;
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
    output_directory.join(format!("{}.json", trace_id))
}
