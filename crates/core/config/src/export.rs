//! Export policy configuration.

use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportConfig {
    pub graph_schema_version: String,
    pub allow_active_trace_snapshot: bool,
    pub output_directory: PathBuf,
    pub payload_bytes_enabled: bool,
    pub payload_text_enabled: bool,
}
