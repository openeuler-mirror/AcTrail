use std::path::PathBuf;
use std::time::Duration;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxAlertSynchronous {
    Normal,
    Full,
}

impl SandboxAlertSynchronous {
    pub(crate) const fn pragma_value(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Full => "FULL",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxAlertSqliteConfig {
    pub path: PathBuf,
    pub schema_version: u32,
    pub create_parent_directory: bool,
    pub busy_timeout: Duration,
    pub writer_queue_capacity: u32,
    pub transaction_max_alerts: u32,
    pub flush_interval: Duration,
    pub retention_max_alerts: u64,
    pub capacity_max_bytes: u64,
    pub synchronous: SandboxAlertSynchronous,
    pub wal_autocheckpoint_pages: u32,
    pub shutdown_drain_timeout: Duration,
    pub writer_thread_stack_bytes: usize,
    pub read_limit_max: u32,
}

impl SandboxAlertSqliteConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.path.is_absolute() || self.path.file_name().is_none() {
            return Err("sandbox alert database path must be an absolute file path".to_string());
        }
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(format!(
                "unsupported sandbox alert schema version {}; expected {CURRENT_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.busy_timeout.is_zero()
            || self.flush_interval.is_zero()
            || self.shutdown_drain_timeout.is_zero()
        {
            return Err("sandbox alert timeouts and intervals must be positive".to_string());
        }
        if self.writer_queue_capacity == 0
            || self.transaction_max_alerts == 0
            || self.retention_max_alerts == 0
            || self.capacity_max_bytes == 0
            || self.wal_autocheckpoint_pages == 0
            || self.read_limit_max == 0
        {
            return Err("sandbox alert capacities and limits must be positive".to_string());
        }
        if self.transaction_max_alerts > self.writer_queue_capacity {
            return Err(
                "sandbox alert transaction_max_alerts must not exceed writer_queue_capacity"
                    .to_string(),
            );
        }
        if u64::from(self.transaction_max_alerts) > self.retention_max_alerts {
            return Err(
                "sandbox alert retention_max_alerts must not be lower than transaction_max_alerts"
                    .to_string(),
            );
        }
        if self.writer_thread_stack_bytes < 64 * 1024 {
            return Err(
                "sandbox alert writer_thread_stack_bytes must be at least 65536".to_string(),
            );
        }
        Ok(())
    }
}
