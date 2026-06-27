//! Resolve operator eBPF settings against the host runtime.
//!
//! When `ebpf.enabled = "auto"`, the daemon probes the host at startup: if the
//! host can run eBPF (BTF present, root, tracefs writable) the collector stays
//! enabled; otherwise the daemon prints `actraild ebpf auto-degraded: ...` and
//! continues without eBPF collection instead of refusing to start. `true` and
//! `false` are honored as-is.

use config_core::daemon::{EbpfCollectorConfig, EbpfEnabledMode};
use ebpf_collector::capability_probe::probe;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EbpfResolution {
    pub config: EbpfCollectorConfig,
    pub auto_degraded: bool,
    pub degrade_detail: Option<String>,
}

pub fn resolve_ebpf_collector_config(mut config: EbpfCollectorConfig) -> EbpfResolution {
    match config.enabled_mode {
        EbpfEnabledMode::True => EbpfResolution {
            config,
            auto_degraded: false,
            degrade_detail: None,
        },
        EbpfEnabledMode::False => {
            config.enabled = false;
            EbpfResolution {
                config,
                auto_degraded: false,
                degrade_detail: None,
            }
        }
        EbpfEnabledMode::Auto => {
            let probe_result = probe();
            if let Some(reason) = probe_result.reason_unavailable {
                config.enabled = false;
                EbpfResolution {
                    config,
                    auto_degraded: true,
                    degrade_detail: Some(format!(
                        "{reason}; continuing without host eBPF collection"
                    )),
                }
            } else {
                config.enabled = true;
                EbpfResolution {
                    config,
                    auto_degraded: false,
                    degrade_detail: None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config_core::daemon::{EbpfCollectorConfig, EbpfEnabledMode, MemlockRlimit};

    fn sample_config(mode: EbpfEnabledMode) -> EbpfCollectorConfig {
        EbpfCollectorConfig {
            enabled_mode: mode,
            enabled: matches!(mode, EbpfEnabledMode::True),
            memlock_rlimit: MemlockRlimit::Inherit,
            tracked_process_max_entries: 64,
            pending_operation_max_entries: 64,
            suppressed_fd_max_entries: 64,
            suppressed_fd_index_slots_per_process: 64,
            event_ring_buffer_max_bytes: 4096,
            file_path_capture_enabled: false,
            file_path_max_bytes: 255,
        }
    }

    #[test]
    fn explicit_false_disables_ebpf() {
        let resolution = resolve_ebpf_collector_config(sample_config(EbpfEnabledMode::False));
        assert!(!resolution.config.enabled);
        assert!(!resolution.auto_degraded);
    }

    #[test]
    fn explicit_true_keeps_ebpf_enabled() {
        let resolution = resolve_ebpf_collector_config(sample_config(EbpfEnabledMode::True));
        assert!(resolution.config.enabled);
        assert!(!resolution.auto_degraded);
    }

    #[test]
    fn auto_follows_probe_result() {
        let resolution = resolve_ebpf_collector_config(sample_config(EbpfEnabledMode::Auto));
        let probe = probe();
        assert_eq!(resolution.config.enabled, probe.reason_unavailable.is_none());
        assert_eq!(resolution.auto_degraded, probe.reason_unavailable.is_some());
        if let Some(reason) = probe.reason_unavailable {
            assert_eq!(
                resolution.degrade_detail,
                Some(format!(
                    "{reason}; continuing without host eBPF collection"
                ))
            );
        }
    }
}
