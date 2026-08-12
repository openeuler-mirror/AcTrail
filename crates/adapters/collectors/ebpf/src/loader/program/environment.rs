//! Kernel environment checks and attach diagnostics.

use config_core::daemon::MemlockRlimit;

use crate::capability_probe;
use crate::loader::LoaderError;

const PERF_EVENT_PARANOID_PATH: &str = "/proc/sys/kernel/perf_event_paranoid";
const UNPRIVILEGED_BPF_DISABLED_PATH: &str = "/proc/sys/kernel/unprivileged_bpf_disabled";

pub(super) fn ensure_tracefs_control() -> Result<(), LoaderError> {
    let tracefs_control = capability_probe::tracefs_control_state();
    if tracefs_control.writable {
        return Ok(());
    }
    Err(LoaderError::new(
        "tracefs_control",
        tracefs_control
            .reason_unavailable
            .unwrap_or_else(|| "tracefs control mount is not writable".to_string()),
    ))
}

pub(super) fn attach_environment_description() -> String {
    let tracefs_control = capability_probe::tracefs_control_state();
    let tracefs = if tracefs_control.writable {
        "tracefs_control=writable".to_string()
    } else {
        format!(
            "tracefs_control={}",
            tracefs_control
                .reason_unavailable
                .unwrap_or_else(|| "not writable".to_string())
        )
    };
    format!(
        "{tracefs}; perf_event_paranoid={}; unprivileged_bpf_disabled={}",
        read_kernel_setting(PERF_EVENT_PARANOID_PATH),
        read_kernel_setting(UNPRIVILEGED_BPF_DISABLED_PATH)
    )
}

pub(super) fn apply_memlock_rlimit(limit: MemlockRlimit) -> Result<(), LoaderError> {
    let Some(resource_limit) = resource_limit(limit) else {
        return Ok(());
    };
    let result = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &resource_limit) };
    if result != 0 {
        return Err(LoaderError::new(
            "set_memlock_rlimit",
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(())
}

fn read_kernel_setting(path: &str) -> String {
    std::fs::read_to_string(path)
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|error| format!("unreadable({error})"))
}

fn resource_limit(limit: MemlockRlimit) -> Option<libc::rlimit> {
    match limit {
        MemlockRlimit::Inherit => None,
        MemlockRlimit::Unlimited => Some(libc::rlimit {
            rlim_cur: libc::RLIM_INFINITY,
            rlim_max: libc::RLIM_INFINITY,
        }),
        MemlockRlimit::Bytes(bytes) => Some(libc::rlimit {
            rlim_cur: bytes as libc::rlim_t,
            rlim_max: bytes as libc::rlim_t,
        }),
    }
}
