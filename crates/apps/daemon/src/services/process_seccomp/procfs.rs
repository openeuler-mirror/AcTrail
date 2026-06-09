//! Procfs helpers for process-control seccomp observations.

use std::path::Path;

use control_contract::reply::ControlError;

use crate::services::seccomp_notify::target_exited;

pub(super) fn parent_pid(pid: u32) -> Result<Option<u32>, ControlError> {
    let raw = match std::fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(raw) => raw,
        Err(error) if target_exited(&error) => return Ok(None),
        Err(error) => {
            return Err(ControlError::new(
                "process_seccomp_procfs",
                error.to_string(),
            ));
        }
    };
    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("PPid:") {
            return value.trim().parse::<u32>().map(Some).map_err(|error| {
                ControlError::new("process_seccomp_procfs", format!("parse PPid: {error}"))
            });
        }
    }
    Err(ControlError::new(
        "process_seccomp_procfs",
        format!("missing PPid for pid {pid}"),
    ))
}

pub(super) fn absolute_exec_path_missing(pid: u32, path: &str) -> bool {
    let Some(relative_path) = path.strip_prefix('/') else {
        return false;
    };
    let path_in_target_root = Path::new("/proc")
        .join(pid.to_string())
        .join("root")
        .join(relative_path);
    matches!(
        std::fs::metadata(path_in_target_root),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound || target_exited(&error)
    )
}
