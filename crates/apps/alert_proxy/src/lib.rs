//! Standalone alert forwarding proxy composition root.

mod broadcaster;
mod daemon_ingress;
mod diagnostics;
mod registry;
mod subscriber;

mod startup;

pub use startup::{AlertProxyBootstrap, AlertProxyConfig, AlertProxyRuntime};

pub fn report_startup_failure(error: &dyn std::fmt::Display) {
    diagnostics::ProxyDiagnostics::startup_failed(error);
}
