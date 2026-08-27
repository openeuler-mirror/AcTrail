use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::SandboxLinuxError;

const DEFAULT_TRACKED_PROCESS_CAPACITY: u32 = 16_384;
const DEFAULT_PENDING_IO_CAPACITY: u32 = 32_768;
const DEFAULT_AGGREGATE_CAPACITY: u32 = 4_096;
const DEFAULT_OOM_EVENT_CAPACITY: u32 = 256;
const MAX_MAP_CAPACITY: u32 = 1_048_576;
const DEFAULT_ROOT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const MIN_ROOT_REFRESH_INTERVAL: Duration = Duration::from_millis(10);
const MAX_ROOT_REFRESH_INTERVAL: Duration = Duration::from_secs(3_600);

/// Linux collector settings validated before any kernel resources are created.
#[derive(Clone, Debug)]
pub struct SandboxLinuxConfig {
    pub(crate) root_process_names: Vec<[u8; 16]>,
    pub(crate) procfs_root: PathBuf,
    pub(crate) tracked_process_capacity: u32,
    pub(crate) pending_io_capacity: u32,
    pub(crate) aggregate_capacity: u32,
    pub(crate) oom_event_capacity: u32,
    pub(crate) require_initial_root: bool,
    pub(crate) root_refresh_interval: Duration,
}

impl SandboxLinuxConfig {
    pub fn new<I, S>(root_process_names: I) -> Result<Self, SandboxLinuxError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let names = root_process_names
            .into_iter()
            .map(|name| Self::encode_process_name(name.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        if names.is_empty() {
            return Err(SandboxLinuxError::new(
                "validate_config",
                "at least one root process name is required",
            ));
        }
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != names.len() {
            return Err(SandboxLinuxError::new(
                "validate_config",
                "root process names must not contain duplicates",
            ));
        }
        Ok(Self {
            root_process_names: names,
            procfs_root: PathBuf::from("/proc"),
            tracked_process_capacity: DEFAULT_TRACKED_PROCESS_CAPACITY,
            pending_io_capacity: DEFAULT_PENDING_IO_CAPACITY,
            aggregate_capacity: DEFAULT_AGGREGATE_CAPACITY,
            oom_event_capacity: DEFAULT_OOM_EVENT_CAPACITY,
            require_initial_root: true,
            root_refresh_interval: DEFAULT_ROOT_REFRESH_INTERVAL,
        })
    }

    pub fn with_procfs_root(mut self, root: impl AsRef<Path>) -> Result<Self, SandboxLinuxError> {
        let root = root.as_ref();
        if !root.is_absolute() {
            return Err(SandboxLinuxError::new(
                "validate_config",
                "procfs root must be absolute",
            ));
        }
        self.procfs_root = root.to_path_buf();
        Ok(self)
    }

    pub fn with_map_capacities(
        mut self,
        tracked_processes: u32,
        pending_io: u32,
        aggregates: u32,
    ) -> Result<Self, SandboxLinuxError> {
        Self::validate_capacity("tracked process", tracked_processes)?;
        Self::validate_capacity("pending I/O", pending_io)?;
        Self::validate_capacity("aggregate", aggregates)?;
        self.tracked_process_capacity = tracked_processes;
        self.pending_io_capacity = pending_io;
        self.aggregate_capacity = aggregates;
        Ok(self)
    }

    pub fn with_oom_event_capacity(mut self, capacity: u32) -> Result<Self, SandboxLinuxError> {
        Self::validate_capacity("OOM event", capacity)?;
        self.oom_event_capacity = capacity;
        Ok(self)
    }

    pub fn with_initial_root_required(mut self, required: bool) -> Self {
        self.require_initial_root = required;
        self
    }

    pub fn with_root_refresh_interval(
        mut self,
        interval: Duration,
    ) -> Result<Self, SandboxLinuxError> {
        if !(MIN_ROOT_REFRESH_INTERVAL..=MAX_ROOT_REFRESH_INTERVAL).contains(&interval) {
            return Err(SandboxLinuxError::new(
                "validate_config",
                format!(
                    "root refresh interval must be between {:?} and {:?}",
                    MIN_ROOT_REFRESH_INTERVAL, MAX_ROOT_REFRESH_INTERVAL
                ),
            ));
        }
        self.root_refresh_interval = interval;
        Ok(self)
    }

    pub fn procfs_root(&self) -> &Path {
        &self.procfs_root
    }

    fn encode_process_name(name: &str) -> Result<[u8; 16], SandboxLinuxError> {
        let raw = name.as_bytes();
        if raw.is_empty() || raw.len() > 15 || raw.contains(&0) {
            return Err(SandboxLinuxError::new(
                "validate_config",
                format!("root process name must contain 1..=15 non-NUL UTF-8 bytes: {name:?}"),
            ));
        }
        let mut encoded = [0_u8; 16];
        encoded[..raw.len()].copy_from_slice(raw);
        Ok(encoded)
    }

    fn validate_capacity(label: &str, value: u32) -> Result<(), SandboxLinuxError> {
        if value == 0 || value > MAX_MAP_CAPACITY {
            return Err(SandboxLinuxError::new(
                "validate_config",
                format!("{label} map capacity must be between 1 and {MAX_MAP_CAPACITY}"),
            ));
        }
        Ok(())
    }
}
