use super::CgroupV2BindingError;
use model_core::process::HostProcessCoordinates;
use std::{fs, path::Path};

pub(super) struct ProcfsProcessVerifier;
impl ProcfsProcessVerifier {
    pub(super) fn require_expected_start(
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), CgroupV2BindingError> {
        if coordinates.start_time_ticks == 0 {
            Err(CgroupV2BindingError::ExpectedStartTimeMissing)
        } else {
            Ok(())
        }
    }

    pub(super) fn require_start_time(
        procfs_root: &Path,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), CgroupV2BindingError> {
        let raw = Self::read_proc_file(procfs_root, coordinates.pid, "stat")?;
        let actual = Self::parse_proc_start_time(&raw)?;
        if actual == coordinates.start_time_ticks {
            Ok(())
        } else {
            Err(CgroupV2BindingError::PidReuse)
        }
    }

    pub(super) fn parse_proc_start_time(raw: &str) -> Result<u64, CgroupV2BindingError> {
        let close = raw.rfind(')').ok_or_else(|| {
            CgroupV2BindingError::Malformed("proc stat is missing command terminator".to_string())
        })?;
        let tail = raw.get(close + 1..).ok_or_else(|| {
            CgroupV2BindingError::Malformed("proc stat command offset overflow".to_string())
        })?;
        tail.split_whitespace()
            .nth(19)
            .ok_or_else(|| {
                CgroupV2BindingError::Malformed("proc stat is missing start time".to_string())
            })?
            .parse::<u64>()
            .map_err(|error| {
                CgroupV2BindingError::Malformed(format!("invalid start time: {error}"))
            })
    }

    pub(super) fn read_proc_file(
        procfs_root: &Path,
        pid: u32,
        name: &'static str,
    ) -> Result<String, CgroupV2BindingError> {
        let path = procfs_root.join(pid.to_string()).join(name);
        fs::read_to_string(&path).map_err(|source| CgroupV2BindingError::Io {
            operation: "read process identity",
            path,
            source,
        })
    }
}
