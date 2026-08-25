use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::daemon::{SandboxAlertsConfig, SandboxAlertsSynchronousConfig};

use super::{require_positive_u32, require_positive_u64};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SandboxAlertsDocument {
    pub enabled: bool,
    pub path: String,
    pub schema_version: u32,
    pub create_parent_directory: bool,
    pub busy_timeout_ms: u64,
    pub writer_queue_capacity: u32,
    pub transaction_max_alerts: u32,
    pub flush_interval_ms: u64,
    pub retention_max_alerts: u64,
    pub capacity_max_bytes: u64,
    pub synchronous: String,
    pub wal_autocheckpoint_pages: u32,
    pub shutdown_drain_timeout_ms: u64,
    pub writer_thread_stack_bytes: usize,
    pub read_limit_max: u32,
}

impl Default for SandboxAlertsDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "/var/lib/actrail/sandbox-alerts.sqlite".to_string(),
            schema_version: 2,
            create_parent_directory: true,
            busy_timeout_ms: 5_000,
            writer_queue_capacity: 1_024,
            transaction_max_alerts: 256,
            flush_interval_ms: 250,
            retention_max_alerts: 1_000_000,
            capacity_max_bytes: 1_073_741_824,
            synchronous: "normal".to_string(),
            wal_autocheckpoint_pages: 1_000,
            shutdown_drain_timeout_ms: 10_000,
            writer_thread_stack_bytes: 524_288,
            read_limit_max: 10_000,
        }
    }
}

impl SandboxAlertsDocument {
    pub(super) fn from_config(config: &SandboxAlertsConfig) -> Self {
        Self {
            enabled: config.enabled,
            path: config.path.display().to_string(),
            schema_version: config.schema_version,
            create_parent_directory: config.create_parent_directory,
            busy_timeout_ms: config.busy_timeout_ms,
            writer_queue_capacity: config.writer_queue_capacity,
            transaction_max_alerts: config.transaction_max_alerts,
            flush_interval_ms: config.flush_interval_ms,
            retention_max_alerts: config.retention_max_alerts,
            capacity_max_bytes: config.capacity_max_bytes,
            synchronous: match config.synchronous {
                SandboxAlertsSynchronousConfig::Normal => "normal",
                SandboxAlertsSynchronousConfig::Full => "full",
            }
            .to_string(),
            wal_autocheckpoint_pages: config.wal_autocheckpoint_pages,
            shutdown_drain_timeout_ms: config.shutdown_drain_timeout_ms,
            writer_thread_stack_bytes: config.writer_thread_stack_bytes,
            read_limit_max: config.read_limit_max,
        }
    }

    pub(super) fn to_config(&self) -> Result<SandboxAlertsConfig, String> {
        let path = PathBuf::from(&self.path);
        if !path.is_absolute() || path.file_name().is_none() {
            return Err("sandbox_alerts.path must be an absolute file path".to_string());
        }
        if self.schema_version != 2 {
            return Err("sandbox_alerts.schema_version must be 2".to_string());
        }
        let writer_queue_capacity = require_positive_u32(
            "sandbox_alerts.writer_queue_capacity",
            self.writer_queue_capacity,
        )?;
        let transaction_max_alerts = require_positive_u32(
            "sandbox_alerts.transaction_max_alerts",
            self.transaction_max_alerts,
        )?;
        if transaction_max_alerts > writer_queue_capacity {
            return Err(
                "sandbox_alerts.transaction_max_alerts must not exceed writer_queue_capacity"
                    .to_string(),
            );
        }
        if u64::from(transaction_max_alerts) > self.retention_max_alerts {
            return Err(
                "sandbox_alerts.retention_max_alerts must not be lower than transaction_max_alerts"
                    .to_string(),
            );
        }
        if self.writer_thread_stack_bytes < 65_536 {
            return Err(
                "sandbox_alerts.writer_thread_stack_bytes must be at least 65536".to_string(),
            );
        }
        let synchronous = match self.synchronous.as_str() {
            "normal" => SandboxAlertsSynchronousConfig::Normal,
            "full" => SandboxAlertsSynchronousConfig::Full,
            other => {
                return Err(format!(
                    "sandbox_alerts.synchronous must be normal or full, got {other}"
                ));
            }
        };
        Ok(SandboxAlertsConfig {
            enabled: self.enabled,
            path,
            schema_version: self.schema_version,
            create_parent_directory: self.create_parent_directory,
            busy_timeout_ms: require_positive_u64(
                "sandbox_alerts.busy_timeout_ms",
                self.busy_timeout_ms,
            )?,
            writer_queue_capacity,
            transaction_max_alerts,
            flush_interval_ms: require_positive_u64(
                "sandbox_alerts.flush_interval_ms",
                self.flush_interval_ms,
            )?,
            retention_max_alerts: require_positive_u64(
                "sandbox_alerts.retention_max_alerts",
                self.retention_max_alerts,
            )?,
            capacity_max_bytes: require_positive_u64(
                "sandbox_alerts.capacity_max_bytes",
                self.capacity_max_bytes,
            )?,
            synchronous,
            wal_autocheckpoint_pages: require_positive_u32(
                "sandbox_alerts.wal_autocheckpoint_pages",
                self.wal_autocheckpoint_pages,
            )?,
            shutdown_drain_timeout_ms: require_positive_u64(
                "sandbox_alerts.shutdown_drain_timeout_ms",
                self.shutdown_drain_timeout_ms,
            )?,
            writer_thread_stack_bytes: self.writer_thread_stack_bytes,
            read_limit_max: require_positive_u32(
                "sandbox_alerts.read_limit_max",
                self.read_limit_max,
            )?,
        })
    }
}
