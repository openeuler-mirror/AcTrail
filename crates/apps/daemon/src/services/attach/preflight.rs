//! Daemon-start eBPF load preflight.

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
    pub(crate) fn preflight_host_ebpf(&mut self) {
        self.host_ebpf_preflight.clear();
        let profiles = self
            .profiles
            .capture_profiles()
            .map(|(name, profile)| {
                (
                    name.clone(),
                    profile.supports_host_ebpf_observation(),
                    profile.capabilities.clone(),
                )
            })
            .collect::<Vec<_>>();

        for (profile_name, uses_host_ebpf, capabilities) in profiles {
            let report = if uses_host_ebpf {
                match self.collector.preflight_capability_requests(&capabilities) {
                    Ok(()) => {
                        self.log_diagnostic(
                            DiagnosticLogLevel::Info,
                            format_args!(
                                "host_ebpf_preflight available profile={}",
                                profile_name.as_str()
                            ),
                        );
                        EbpfPreflightReport::available()
                    }
                    Err(error) => {
                        self.log_diagnostic(
                            DiagnosticLogLevel::Info,
                            format_args!(
                                "host_ebpf_preflight unavailable profile={} stage={} message={}",
                                profile_name.as_str(),
                                error.stage,
                                error.message
                            ),
                        );
                        EbpfPreflightReport::unavailable(error.stage, error.message)
                    }
                }
            } else {
                EbpfPreflightReport::unavailable(
                    "profile",
                    "capture profile does not request host eBPF observation",
                )
            };
            self.host_ebpf_preflight.insert(profile_name, report);
        }
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
