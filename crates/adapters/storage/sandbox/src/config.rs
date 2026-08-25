use std::path::PathBuf;
use std::time::Duration;

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEvidenceSynchronous {
    Normal,
    Full,
}

impl SandboxEvidenceSynchronous {
    pub(crate) const fn pragma_value(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Full => "FULL",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxEvidenceSqliteConfig {
    /// Absolute path to the database file, separate from the main AcTrail storage.
    pub path: PathBuf,
    /// Required on-disk schema version. Only `CURRENT_SCHEMA_VERSION` is accepted.
    pub schema_version: u32,
    pub create_parent_directory: bool,
    pub busy_timeout: Duration,
    pub writer_queue_capacity: u32,
    pub batch_max_observations: u32,
    pub transaction_max_batches: u32,
    pub flush_interval: Duration,
    pub retention_max_observations: u64,
    pub capacity_max_bytes: u64,
    pub synchronous: SandboxEvidenceSynchronous,
    pub wal_autocheckpoint_pages: u32,
    pub shutdown_drain_timeout: Duration,
    pub writer_thread_stack_bytes: usize,
    pub read_limit_max: u32,
}

impl SandboxEvidenceSqliteConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.path.is_absolute() {
            return Err("sandbox evidence database path must be absolute".to_string());
        }
        if self.path.file_name().is_none() {
            return Err("sandbox evidence database path must name a file".to_string());
        }
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(format!(
                "unsupported sandbox evidence schema version {}; expected {CURRENT_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.busy_timeout.is_zero()
            || self.flush_interval.is_zero()
            || self.shutdown_drain_timeout.is_zero()
        {
            return Err("sandbox evidence timeouts and intervals must be positive".to_string());
        }
        if self.writer_queue_capacity == 0
            || self.batch_max_observations == 0
            || self.transaction_max_batches == 0
            || self.retention_max_observations == 0
            || self.capacity_max_bytes == 0
            || self.wal_autocheckpoint_pages == 0
            || self.read_limit_max == 0
        {
            return Err("sandbox evidence capacities and limits must be positive".to_string());
        }
        if self.transaction_max_batches > self.writer_queue_capacity {
            return Err(
                "sandbox evidence transaction_max_batches must not exceed writer_queue_capacity"
                    .to_string(),
            );
        }
        if self.writer_thread_stack_bytes < 64 * 1024 {
            return Err(
                "sandbox evidence writer_thread_stack_bytes must be at least 65536".to_string(),
            );
        }
        Ok(())
    }
}
