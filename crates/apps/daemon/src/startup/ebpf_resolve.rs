//! Resolve operator eBPF settings against the host runtime.
//!
//! When `ebpf.enabled = "auto"`, the daemon probes the host at startup: if the
//! host can run eBPF (BTF present, root, tracefs writable) the collector stays
//! enabled; otherwise the daemon logs `actraild ebpf auto-degraded` and
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
