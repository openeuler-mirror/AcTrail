//! Concrete daemon services backed by procfs bootstrap and sqlite storage.

pub(crate) mod application_protocol;
pub(crate) mod attach;
#[path = "services/logging/diagnostic.rs"]
pub(crate) mod diagnostic_logging;
pub(crate) mod enforcement;
pub(crate) mod live;
pub(crate) mod payload;
pub(crate) mod payload_gate;
pub(crate) mod process_seccomp;
pub(crate) mod resource_metrics;
pub(crate) mod seccomp_notify;
pub(crate) mod seccomp_socket;
pub(crate) mod seccomp_tls;
pub(crate) mod semantic_actions;
pub(crate) mod wiring;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use wiring::{build_runtime_wiring, build_runtime_wiring_with_provider_rule_set};
