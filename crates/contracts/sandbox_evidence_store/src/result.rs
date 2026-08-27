use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEvidenceSourceError {
    ZeroGatewayId,
    ZeroSbId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEvidenceBatchError {
    Empty,
    ObservationCountOverflow,
    InvalidObservationIndex(u32),
    IndicesNotStrictlyIncreasing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEvidenceAdmission {
    Accepted {
        observation_count: u32,
    },
    TooLarge {
        observation_count: u32,
        max_observations: u32,
    },
    Full {
        observation_count: u32,
    },
    Closed {
        observation_count: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxEvidenceReadError {
    pub code: String,
    pub message: String,
}

impl SandboxEvidenceReadError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SandboxEvidenceReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SandboxEvidenceReadError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxEvidenceShutdownError {
    pub code: String,
    pub message: String,
}

impl SandboxEvidenceShutdownError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SandboxEvidenceShutdownError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SandboxEvidenceShutdownError {}
