//! Connection and descriptor budgets for process-local TLS-sync transports.

use config_core::daemon::PayloadTlsConfig;
use control_contract::reply::ControlError;

pub(super) fn client_limit(
    config: &PayloadTlsConfig,
    active_trace_max: u32,
) -> Result<usize, ControlError> {
    (active_trace_max as usize)
        .checked_mul(config.sync_trace_connection_max as usize)
        .and_then(|count| count.checked_add(config.sync_pending_connection_max as usize))
        .ok_or_else(|| {
            ControlError::new(
                "tls_sync_config",
                "TLS-sync connection budget exceeds this platform",
            )
        })
}

pub(in crate::services) fn configure_fd_limit(
    config: &PayloadTlsConfig,
    active_trace_max: u32,
) -> Result<(), ControlError> {
    if !super::enabled(config) {
        return Ok(());
    }
    let required = client_limit(config, active_trace_max)?
        .checked_add(config.sync_plan_queue_capacity as usize)
        .and_then(|count| count.checked_add(config.sync_plan_queue_capacity as usize))
        .and_then(|count| count.checked_add(config.sync_fd_reserve as usize))
        .ok_or_else(|| {
            ControlError::new(
                "tls_sync_config",
                "TLS-sync fd budget exceeds this platform",
            )
        })?;
    let required = required as libc::rlim_t;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } < 0 {
        return Err(limit_error(required));
    }
    if limit.rlim_cur >= required {
        return Ok(());
    }
    limit.rlim_cur = required;
    limit.rlim_max = limit.rlim_max.max(required);
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } < 0 {
        return Err(limit_error(required));
    }
    Ok(())
}

fn limit_error(required: libc::rlim_t) -> ControlError {
    ControlError::new(
        "tls_sync_fd_limit",
        format!(
            "TLS-sync requires RLIMIT_NOFILE >= {required}: {}",
            std::io::Error::last_os_error()
        ),
    )
}
