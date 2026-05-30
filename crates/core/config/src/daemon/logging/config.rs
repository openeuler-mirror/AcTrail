//! Daemon diagnostic logging configuration.

use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DiagnosticLogLevel {
    Off,
    Info,
    Debug,
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
