//! Command results returned to the short-lived actrail-sb control CLI.

use crate::SandboxEndpoint;

pub const MAX_SANDBOX_CONTROL_REJECTION_REASON_BYTES: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxControlResponse {
    Connect(SandboxConnectResponse),
    Rejected(SandboxControlRejection),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxConnectResponse {
    endpoint: SandboxEndpoint,
    sb_id: u32,
    connection_generation: u64,
    reused: bool,
}

impl SandboxConnectResponse {
    pub const fn new(
        endpoint: SandboxEndpoint,
        sb_id: u32,
        connection_generation: u64,
        reused: bool,
    ) -> Self {
        Self {
            endpoint,
            sb_id,
            connection_generation,
            reused,
        }
    }

    pub const fn endpoint(self) -> SandboxEndpoint {
        self.endpoint
    }

    pub const fn sb_id(self) -> u32 {
        self.sb_id
    }

    pub const fn connection_generation(self) -> u64 {
        self.connection_generation
    }

    pub const fn reused(self) -> bool {
        self.reused
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxControlRejectionCode {
    InvalidRequest,
    Busy,
    ConnectFailed,
    HandshakeFailed,
    ShuttingDown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxControlRejection {
    code: SandboxControlRejectionCode,
    message: String,
}

impl SandboxControlRejection {
    pub fn new(
        code: SandboxControlRejectionCode,
        message: impl Into<String>,
    ) -> Result<Self, SandboxControlRejectionError> {
        let message = message.into();
        if message.is_empty() {
            return Err(SandboxControlRejectionError::EmptyReason);
        }
        if message.len() > MAX_SANDBOX_CONTROL_REJECTION_REASON_BYTES {
            return Err(SandboxControlRejectionError::ReasonTooLong);
        }
        Ok(Self { code, message })
    }

    pub const fn code(&self) -> SandboxControlRejectionCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxControlRejectionError {
    EmptyReason,
    ReasonTooLong,
}

impl std::fmt::Display for SandboxControlRejectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyReason => formatter.write_str("sandbox control rejection reason is empty"),
            Self::ReasonTooLong => {
                formatter.write_str("sandbox control rejection reason exceeds protocol limit")
            }
        }
    }
}

impl std::error::Error for SandboxControlRejectionError {}
