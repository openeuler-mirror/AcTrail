//! Daemon-start eBPF load preflight.

use std::collections::BTreeMap;
use std::time::Instant;

use config_core::daemon::DiagnosticLogLevel;
use ebpf_collector::EbpfPreflightKey;
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
        let started_at = Instant::now();
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
        let mut cached_reports =
            BTreeMap::<EbpfPreflightKey, (ProfileName, EbpfPreflightReport)>::new();
        let mut host_ebpf_profiles = 0_usize;
        let mut cache_hits = 0_usize;

        for (profile_name, uses_host_ebpf, capabilities) in profiles {
            let report = if uses_host_ebpf {
                host_ebpf_profiles = host_ebpf_profiles.saturating_add(1);
                let key = self.collector.preflight_key(&capabilities);
                if let Some((source_profile, report)) = cached_reports.get(&key) {
                    cache_hits = cache_hits.saturating_add(1);
                    if report.available {
                        self.log_diagnostic(
                            DiagnosticLogLevel::Info,
                            format_args!(
                                "host_ebpf_preflight available profile={} cache=hit source_profile={}",
                                profile_name.as_str(),
                                source_profile.as_str()
                            ),
                        );
                    } else {
                        self.log_diagnostic(
                            DiagnosticLogLevel::Info,
                            format_args!(
                                "host_ebpf_preflight unavailable profile={} cache=hit source_profile={} stage={} message={}",
                                profile_name.as_str(),
                                source_profile.as_str(),
                                report.failure_stage.as_deref().unwrap_or("unknown"),
                                report.failure_message.as_deref().unwrap_or("unknown")
                            ),
                        );
                    }
                    report.clone()
                } else {
                    let plan_started_at = Instant::now();
                    let report = match self.collector.preflight_capability_requests(&capabilities) {
                        Ok(()) => {
                            self.log_diagnostic(
                                DiagnosticLogLevel::Info,
                                format_args!(
                                    "host_ebpf_preflight available profile={} cache=miss elapsed_ms={}",
                                    profile_name.as_str(),
                                    plan_started_at.elapsed().as_millis()
                                ),
                            );
                            EbpfPreflightReport::available()
                        }
                        Err(error) => {
                            self.log_diagnostic(
                                DiagnosticLogLevel::Info,
                                format_args!(
                                    "host_ebpf_preflight unavailable profile={} cache=miss elapsed_ms={} stage={} message={}",
                                    profile_name.as_str(),
                                    plan_started_at.elapsed().as_millis(),
                                    error.stage,
                                    error.message
                                ),
                            );
                            EbpfPreflightReport::unavailable(error.stage, error.message)
                        }
                    };
                    cached_reports.insert(key, (profile_name.clone(), report.clone()));
                    report
                }
            } else {
                EbpfPreflightReport::unavailable(
                    "profile",
                    "capture profile does not request host eBPF observation",
                )
            };
            self.host_ebpf_preflight.insert(profile_name, report);
        }

        self.log_diagnostic(
            DiagnosticLogLevel::Info,
            format_args!(
                "host_ebpf_preflight completed profiles={} unique_plans={} cache_hits={} event_transport={} elapsed_ms={}",
                host_ebpf_profiles,
                cached_reports.len(),
                cache_hits,
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
