use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxResourceAlertConfig {
    pub memory_available_threshold_bytes: u64,
    pub read_interval_threshold_bytes: u64,
    pub write_interval_threshold_bytes: u64,
    pub source_state_capacity: u32,
}

impl SandboxResourceAlertConfig {
    pub(crate) fn validate(self) -> Result<usize, SandboxResourceAlertConfigError> {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxResourceAlertConfigError {
    ZeroMemoryThreshold,
    ZeroReadThreshold,
    ZeroWriteThreshold,
    ZeroSourceStateCapacity,
    SourceStateCapacityOverflow,
}

impl fmt::Display for SandboxResourceAlertConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroMemoryThreshold => "memory available threshold must be positive",
            Self::ZeroReadThreshold => "read interval threshold must be positive",
            Self::ZeroWriteThreshold => "write interval threshold must be positive",
            Self::ZeroSourceStateCapacity => "source state capacity must be positive",
            Self::SourceStateCapacityOverflow => "source state capacity does not fit usize",
        })
    }
}

impl std::error::Error for SandboxResourceAlertConfigError {}
