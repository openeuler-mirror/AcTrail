use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxResourceAlertConfig {
    pub cpu_usage_threshold_basis_points: u16,
    pub memory_available_threshold_bytes: u64,
    pub read_interval_threshold_bytes: u64,
    pub write_interval_threshold_bytes: u64,
    pub source_state_capacity: u32,
}

impl SandboxResourceAlertConfig {
    pub fn from_json(raw: &str) -> Result<Self, SandboxResourceAlertConfigError> {
        let config: Self = serde_json::from_str(raw)
            .map_err(|error| SandboxResourceAlertConfigError::InvalidJson(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_json(self) -> Result<String, SandboxResourceAlertConfigError> {
        serde_json::to_string(&self)
            .map_err(|error| SandboxResourceAlertConfigError::InvalidJson(error.to_string()))
    }

    pub(crate) fn validate(self) -> Result<usize, SandboxResourceAlertConfigError> {
        if !(1..=10_000).contains(&self.cpu_usage_threshold_basis_points) {
            return Err(SandboxResourceAlertConfigError::InvalidCpuThreshold);
        }
        if self.memory_available_threshold_bytes == 0 {
            return Err(SandboxResourceAlertConfigError::ZeroMemoryThreshold);
        }
        if self.read_interval_threshold_bytes == 0 {
            return Err(SandboxResourceAlertConfigError::ZeroReadThreshold);
        }
        if self.write_interval_threshold_bytes == 0 {
            return Err(SandboxResourceAlertConfigError::ZeroWriteThreshold);
        }
        if self.source_state_capacity == 0 {
            return Err(SandboxResourceAlertConfigError::ZeroSourceStateCapacity);
        }
        usize::try_from(self.source_state_capacity)
            .map_err(|_| SandboxResourceAlertConfigError::SourceStateCapacityOverflow)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxResourceAlertConfigError {
    InvalidJson(String),
    InvalidCpuThreshold,
    ZeroMemoryThreshold,
    ZeroReadThreshold,
    ZeroWriteThreshold,
    ZeroSourceStateCapacity,
    SourceStateCapacityOverflow,
}

impl fmt::Display for SandboxResourceAlertConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidJson(message) => {
                return write!(formatter, "invalid config JSON: {message}");
            }
            Self::InvalidCpuThreshold => {
                "CPU usage threshold must be between 1 and 10000 basis points"
            }
            Self::ZeroMemoryThreshold => "memory available threshold must be positive",
            Self::ZeroReadThreshold => "read interval threshold must be positive",
            Self::ZeroWriteThreshold => "write interval threshold must be positive",
            Self::ZeroSourceStateCapacity => "source state capacity must be positive",
            Self::SourceStateCapacityOverflow => "source state capacity does not fit usize",
        })
    }
}

impl std::error::Error for SandboxResourceAlertConfigError {}
