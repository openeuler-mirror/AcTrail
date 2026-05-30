//! Live OTEL JSONL export configuration.

use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveOtelExportConfig {
    pub enabled: bool,
    pub path: PathBuf,
    pub overwrite_enabled: bool,
    pub queue_capacity: u32,
    pub flush_every_spans: u32,
}
