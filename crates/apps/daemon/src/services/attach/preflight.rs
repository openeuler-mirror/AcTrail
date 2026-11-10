//! Prepare the configured eBPF runtime once, then evaluate profile requirements.

use std::time::Instant;

use config_core::daemon::DiagnosticLogLevel;
use model_core::ids::ProfileName;

use super::StorageAttachService;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::services) struct EbpfPreflightReport {
    pub(super) available: bool,
    pub(super) failure_stage: Option<String>,
    pub(super) failure_message: Option<String>,
}

impl EbpfPreflightReport {
    fn available() -> Self {
        Self {
            available: true,
            failure_stage: None,
            failure_message: None,
        }
    }

    fn unavailable(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            available: false,
            failure_stage: Some(stage.into()),
            failure_message: Some(message.into()),
        }
    }
}

impl StorageAttachService {
    pub(in crate::services) fn require_lsm_preflight(
        &self,
    ) -> Result<(), control_contract::reply::ControlError> {
        self.collector.require_lsm_ready().map_err(|error| {
            control_contract::reply::ControlError::new(
                "lsm_preflight",
                format!("{}: {}", error.stage, error.message),
            )
        })
    }

    pub(crate) fn preflight_host_ebpf(&mut self) {
        let started_at = Instant::now();
        // Permission-derived launch candidates share this configuration. They
        // consume the result below and never expand the loading scope.
        let requests = self
            .profiles
            .configured_capture_profiles()
            .flat_map(|(_, profile)| profile.capabilities.iter().cloned())
            .collect::<Vec<_>>();
        let needs_runtime =
            self.collector.has_capability_requests(&requests) || self.lsm_access_control.is_some();
        if needs_runtime {
            self.log_diagnostic(
                DiagnosticLogLevel::Info,
                format_args!("host_ebpf_preparation started"),
            );
        }
        let prepared = self.collector.prepare_capabilities(&requests);
        self.log_diagnostic(
            DiagnosticLogLevel::Info,
            format_args!(
                "host_ebpf_preparation completed requested={} available={} elapsed_ms={}",
                needs_runtime,
                prepared.is_ok(),
                started_at.elapsed().as_millis()
            ),
        );
        self.host_ebpf_preflight.clear();
        let profiles = self
            .profiles
            .capture_profiles()
            .map(|(name, profile)| {
                (
                    name.clone(),
                    self.collector
                        .has_capability_requests(&profile.capabilities),
                    profile.capabilities.clone(),
                )
            })
            .collect::<Vec<_>>();
        let mut host_ebpf_profiles = 0;
        for (name, uses_host_ebpf, capabilities) in profiles {
            let report = if uses_host_ebpf {
                host_ebpf_profiles += 1;
                match self.collector.check_preflight_capabilities(&capabilities) {
                    Ok(()) => EbpfPreflightReport::available(),
                    Err(error) => EbpfPreflightReport::unavailable(error.stage, error.message),
                }
            } else {
                EbpfPreflightReport::unavailable(
                    "profile",
                    "capture profile does not request host eBPF observation",
                )
            };
            self.log_diagnostic(
                DiagnosticLogLevel::Info,
                format_args!(
                    "host_ebpf_preflight profile={} available={} stage={} message={}",
                    name.as_str(),
                    report.available,
                    report.failure_stage.as_deref().unwrap_or("ready"),
                    report.failure_message.as_deref().unwrap_or("")
                ),
            );
            self.host_ebpf_preflight.insert(name, report);
        }
        self.log_diagnostic(
            DiagnosticLogLevel::Info,
            format_args!(
                "host_ebpf_preflight completed profiles={} event_transport={} elapsed_ms={}",
                host_ebpf_profiles,
                self.collector.event_transport(),
                started_at.elapsed().as_millis()
            ),
        );
    }

    pub(crate) fn host_ebpf_preflight_available_for_profile(
        &self,
        profile_name: &ProfileName,
    ) -> bool {
        self.host_ebpf_preflight
            .get(profile_name)
            .is_some_and(|report| report.available)
    }
}
