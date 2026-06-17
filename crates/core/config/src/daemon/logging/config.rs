//! Daemon diagnostic logging configuration.

use std::str::FromStr;

pub const DEFAULT_WORKLOAD_DIAGNOSTICS_ENABLED: bool = false;
pub const DEFAULT_WORKLOAD_DIAGNOSTICS_INTERVAL_MS: u64 = 1000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DiagnosticLogLevel {
    Off,
    Info,
    Debug,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkloadDiagnosticsConfig {
    pub enabled: bool,
    pub interval_ms: u64,
}

impl DiagnosticLogLevel {
    pub fn enables(self, required: Self) -> bool {
        self >= required
    }
}

impl FromStr for DiagnosticLogLevel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            other => Err(format!("expected off, info, or debug, got {other}")),
        }
    }
}
