use super::HostCgroupBindingError;
use model_core::process::HostProcessCoordinates;
use std::{fs, path::Path};

pub(super) struct ProcfsProcessVerifier;
impl ProcfsProcessVerifier {
    pub(super) fn require_expected_start(
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), HostCgroupBindingError> {
        if coordinates.start_time_ticks == 0 {
            Err(HostCgroupBindingError::ExpectedStartTimeMissing)
        } else {
            Ok(())
        }
    }

    pub(super) fn require_start_time(
        procfs_root: &Path,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), HostCgroupBindingError> {
        let raw = Self::read_proc_file(procfs_root, coordinates.pid, "stat")?;
        let actual = Self::parse_proc_start_time(&raw)?;
        if actual == coordinates.start_time_ticks {
            Ok(())
        } else {
            Err(HostCgroupBindingError::PidReuse)
        }
    }

    pub(super) fn parse_proc_start_time(raw: &str) -> Result<u64, HostCgroupBindingError> {
        let close = raw.rfind(')').ok_or_else(|| {
            HostCgroupBindingError::Malformed("proc stat is missing command terminator".to_string())
        })?;
        let tail = raw.get(close + 1..).ok_or_else(|| {
            HostCgroupBindingError::Malformed("proc stat command offset overflow".to_string())
        })?;
        tail.split_whitespace()
            .nth(19)
            .ok_or_else(|| {
                HostCgroupBindingError::Malformed("proc stat is missing start time".to_string())
            })?
            .parse::<u64>()
            .map_err(|error| {
                HostCgroupBindingError::Malformed(format!("invalid start time: {error}"))
            })
    }

    pub(super) fn read_proc_file(
        procfs_root: &Path,
        pid: u32,
        name: &'static str,
    ) -> Result<String, HostCgroupBindingError> {
        let path = procfs_root.join(pid.to_string()).join(name);
        fs::read_to_string(&path).map_err(|source| HostCgroupBindingError::Io {
            operation: "read process identity",
            path,
            source,
        })
    }
}
