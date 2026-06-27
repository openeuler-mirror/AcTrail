//! Launch-time seccomp mode resolution and degradation.

use crate::platform_probe::LaunchPlatformReport;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchSeccompMode {
    Auto,
    Require,
    Skip,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveLaunchSeccomp {
    pub use_seccomp: bool,
    pub payload_socket_enabled: bool,
    pub process_seccomp_enabled: bool,
    pub payload_tls_seccomp_enabled: bool,
    pub degraded: bool,
    pub degrade_detail: Option<String>,
}

pub fn resolve_launch_seccomp(
    mode: LaunchSeccompMode,
    tls_sync_enabled: bool,
    payload_tls_seccomp_enabled: bool,
    payload_socket_enabled: bool,
    process_seccomp_enabled: bool,
    probe: Option<&LaunchPlatformReport>,
) -> Result<EffectiveLaunchSeccomp, String> {
    let seccomp_needed = payload_tls_seccomp_enabled || payload_socket_enabled || process_seccomp_enabled;
    let seccomp_available = probe.is_none_or(|report| report.seccomp_launch_available());

    match mode {
        LaunchSeccompMode::Skip => Ok(EffectiveLaunchSeccomp {
            use_seccomp: false,
            payload_socket_enabled: false,
            process_seccomp_enabled: false,
            payload_tls_seccomp_enabled: false,
            degraded: seccomp_needed,
            degrade_detail: seccomp_needed.then_some(
                "seccomp launch disabled by --seccomp-mode skip".to_string(),
            ),
        }),
        LaunchSeccompMode::Require if seccomp_needed && !seccomp_available => Err(probe
            .map(|report| report.seccomp_launch.detail.clone())
            .unwrap_or_else(|| "seccomp launch path unavailable".to_string())),
        LaunchSeccompMode::Require => Ok(EffectiveLaunchSeccomp {
            use_seccomp: seccomp_needed,
            payload_socket_enabled,
            process_seccomp_enabled,
            payload_tls_seccomp_enabled,
            degraded: false,
            degrade_detail: None,
        }),
        LaunchSeccompMode::Auto if seccomp_needed && !seccomp_available => {
            let detail = probe
                .map(|report| report.seccomp_launch.detail.clone())
                .unwrap_or_else(|| "seccomp launch path unavailable".to_string());
            Ok(EffectiveLaunchSeccomp {
                use_seccomp: false,
                payload_socket_enabled: false,
                process_seccomp_enabled: false,
                payload_tls_seccomp_enabled: false,
                degraded: true,
                degrade_detail: Some(if tls_sync_enabled {
                    format!(
                        "{detail}; continuing with tls-sync-only launch without socket/process seccomp"
                    )
                } else {
                    format!("{detail}; continuing without launch-time seccomp")
                }),
            })
        }
        LaunchSeccompMode::Auto => Ok(EffectiveLaunchSeccomp {
            use_seccomp: seccomp_needed,
            payload_socket_enabled,
            process_seccomp_enabled,
            payload_tls_seccomp_enabled,
            degraded: false,
            degrade_detail: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use linux_platform::capability_probe::CapabilityStatus;

    use super::*;
    use crate::platform_probe::LaunchPlatformReport;

    fn unavailable_probe() -> LaunchPlatformReport {
        LaunchPlatformReport {
            control_socket: CapabilityStatus::ok("control_socket", "ok"),
            tls_sync_socket: CapabilityStatus::ok("tls_sync_socket", "ok"),
            no_new_privs: CapabilityStatus::ok("no_new_privs", "ok"),
            seccomp_launch: CapabilityStatus::unavailable("seccomp_launch", "denied"),
            tls_sync_runtime_library: CapabilityStatus::ok("tls_sync_runtime_library", "ok"),
            daemon: None,
        }
    }

    #[test]
    fn auto_degrades_when_seccomp_unavailable() {
        let effective = resolve_launch_seccomp(
            LaunchSeccompMode::Auto,
            true,
            false,
            true,
            false,
            Some(&unavailable_probe()),
        )
        .unwrap();
        assert!(!effective.use_seccomp);
        assert!(!effective.payload_socket_enabled);
        assert!(effective.degraded);
    }

    #[test]
    fn require_fails_when_seccomp_unavailable() {
        let error = resolve_launch_seccomp(
            LaunchSeccompMode::Require,
            true,
            false,
            true,
            false,
            Some(&unavailable_probe()),
        )
        .unwrap_err();
        assert!(error.contains("denied"));
    }
}
