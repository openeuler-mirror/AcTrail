//! Concrete daemon services backed by procfs bootstrap and storage persistence.

pub(crate) mod application_protocol;
pub(crate) mod attach;
pub(crate) mod command_control;
pub(crate) mod control_runtime;
#[path = "services/logging/diagnostic.rs"]
pub(crate) mod diagnostic_logging;
pub(crate) mod enforcement;
#[path = "services/identity/service.rs"]
pub(crate) mod identity;
pub(crate) mod live;
pub(crate) mod network_control;
pub(crate) mod payload;
pub(crate) mod payload_gate;
pub(crate) mod process_seccomp;
pub(crate) mod resource_metrics;
pub(crate) mod seccomp_notify;
pub(crate) mod seccomp_socket;
pub(crate) mod seccomp_tls;
pub(crate) mod semantic_actions;
#[path = "services/tls_sync/service.rs"]
pub(crate) mod tls_sync;
pub(crate) mod wiring;
#[path = "services/logging/workload.rs"]
pub(crate) mod workload_diagnostics;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use wiring::{build_runtime_wiring, build_runtime_wiring_with_provider_rule_set};
