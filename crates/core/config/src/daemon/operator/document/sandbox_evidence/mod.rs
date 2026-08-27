use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::daemon::{SandboxEvidenceConfig, SandboxEvidenceSynchronousConfig};

use super::{require_positive_u32, require_positive_u64};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SandboxEvidenceDocument {
    pub path: String,
    pub schema_version: u32,
    pub create_parent_directory: bool,
    pub busy_timeout_ms: u64,
    pub writer_queue_capacity: u32,
    pub batch_max_observations: u32,
    pub transaction_max_batches: u32,
    pub flush_interval_ms: u64,
    pub retention_max_observations: u64,
    pub capacity_max_bytes: u64,
    pub synchronous: String,
    pub wal_autocheckpoint_pages: u32,
    pub shutdown_drain_timeout_ms: u64,
    pub writer_thread_stack_bytes: usize,
    pub read_limit_max: u32,
}

impl Default for SandboxEvidenceDocument {
    fn default() -> Self {
        Self {
            path: "/var/lib/actrail/sandbox-evidence.sqlite".to_string(),
            schema_version: 2,
            create_parent_directory: true,
            busy_timeout_ms: 5_000,
            writer_queue_capacity: 1_024,
            batch_max_observations: 1_024,
            transaction_max_batches: 32,
            flush_interval_ms: 250,
            retention_max_observations: 1_000_000,
            capacity_max_bytes: 1_073_741_824,
            synchronous: "normal".to_string(),
            wal_autocheckpoint_pages: 1_000,
            shutdown_drain_timeout_ms: 10_000,
            writer_thread_stack_bytes: 524_288,
            read_limit_max: 10_000,
        }
    }
}

impl SandboxEvidenceDocument {
    pub(super) fn from_config(config: &SandboxEvidenceConfig) -> Self {
        Self {
            path: config.path.display().to_string(),
            schema_version: config.schema_version,
            create_parent_directory: config.create_parent_directory,
            busy_timeout_ms: config.busy_timeout_ms,
            writer_queue_capacity: config.writer_queue_capacity,
            batch_max_observations: config.batch_max_observations,
            transaction_max_batches: config.transaction_max_batches,
            flush_interval_ms: config.flush_interval_ms,
            retention_max_observations: config.retention_max_observations,
            capacity_max_bytes: config.capacity_max_bytes,
            synchronous: match config.synchronous {
                SandboxEvidenceSynchronousConfig::Normal => "normal",
                SandboxEvidenceSynchronousConfig::Full => "full",
            }
            .to_string(),
            wal_autocheckpoint_pages: config.wal_autocheckpoint_pages,
            shutdown_drain_timeout_ms: config.shutdown_drain_timeout_ms,
            writer_thread_stack_bytes: config.writer_thread_stack_bytes,
            read_limit_max: config.read_limit_max,
        }
    }

    pub(super) fn to_config(&self) -> Result<SandboxEvidenceConfig, String> {
        let path = PathBuf::from(&self.path);
        if !path.is_absolute() || path.file_name().is_none() {
            return Err("sandbox_evidence.path must be an absolute file path".to_string());
        }
        if self.schema_version != 2 {
            return Err("sandbox_evidence.schema_version must be 2".to_string());
        }
        let writer_queue_capacity = require_positive_u32(
            "sandbox_evidence.writer_queue_capacity",
            self.writer_queue_capacity,
        )?;
        let transaction_max_batches = require_positive_u32(
            "sandbox_evidence.transaction_max_batches",
            self.transaction_max_batches,
        )?;
        if transaction_max_batches > writer_queue_capacity {
            return Err(
                "sandbox_evidence.transaction_max_batches must not exceed writer_queue_capacity"
                    .to_string(),
            );
        }
        if self.writer_thread_stack_bytes < 65_536 {
            return Err(
                "sandbox_evidence.writer_thread_stack_bytes must be at least 65536".to_string(),
            );
        }
        let synchronous = match self.synchronous.as_str() {
            "normal" => SandboxEvidenceSynchronousConfig::Normal,
            "full" => SandboxEvidenceSynchronousConfig::Full,
            other => {
                return Err(format!(
                    "sandbox_evidence.synchronous must be normal or full, got {other}"
                ));
            }
        };
        Ok(SandboxEvidenceConfig {
            path,
            schema_version: self.schema_version,
            create_parent_directory: self.create_parent_directory,
            busy_timeout_ms: require_positive_u64(
                "sandbox_evidence.busy_timeout_ms",
                self.busy_timeout_ms,
            )?,
            writer_queue_capacity,
            batch_max_observations: require_positive_u32(
                "sandbox_evidence.batch_max_observations",
                self.batch_max_observations,
            )?,
            transaction_max_batches,
            flush_interval_ms: require_positive_u64(
                "sandbox_evidence.flush_interval_ms",
                self.flush_interval_ms,
            )?,
            retention_max_observations: require_positive_u64(
                "sandbox_evidence.retention_max_observations",
                self.retention_max_observations,
            )?,
            capacity_max_bytes: require_positive_u64(
                "sandbox_evidence.capacity_max_bytes",
                self.capacity_max_bytes,
            )?,
            synchronous,
            wal_autocheckpoint_pages: require_positive_u32(
                "sandbox_evidence.wal_autocheckpoint_pages",
                self.wal_autocheckpoint_pages,
            )?,
            shutdown_drain_timeout_ms: require_positive_u64(
                "sandbox_evidence.shutdown_drain_timeout_ms",
                self.shutdown_drain_timeout_ms,
            )?,
            writer_thread_stack_bytes: self.writer_thread_stack_bytes,
            read_limit_max: require_positive_u32(
                "sandbox_evidence.read_limit_max",
                self.read_limit_max,
            )?,
        })
    }
}
