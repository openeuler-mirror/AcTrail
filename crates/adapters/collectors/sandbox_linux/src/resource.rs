use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sandbox_observation::{GuestBootId, GuestResourceSnapshot};

use crate::SandboxLinuxError;
use crate::procfs::ProcfsReader;

/// Reads independent Guest CPU, memory and boot identity snapshots from procfs.
pub struct LinuxResourceReader {
    procfs: ProcfsReader,
    boot_id: GuestBootId,
}

impl LinuxResourceReader {
    pub fn open(procfs_root: PathBuf) -> Result<Self, SandboxLinuxError> {
        let procfs = ProcfsReader::open(procfs_root)?;
        let boot_id = procfs.boot_id()?;
        procfs.cpu_snapshot()?;
        procfs.memory_snapshot()?;
        Ok(Self { procfs, boot_id })
    }

    pub fn sample(&self) -> Result<GuestResourceSnapshot, SandboxLinuxError> {
        let sampled_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| SandboxLinuxError::new("sample_clock", error.to_string()))?
            .as_millis()
            .try_into()
            .map_err(|error| {
                SandboxLinuxError::new("sample_clock", format!("timestamp overflow: {error}"))
            })?;
        let cpu = self.procfs.cpu_snapshot()?;
        let memory = self.procfs.memory_snapshot()?;
        Ok(GuestResourceSnapshot {
            guest_boot_id: self.boot_id,
            sampled_at_ms,
            cpu,
            memory,
        })
    }
}
