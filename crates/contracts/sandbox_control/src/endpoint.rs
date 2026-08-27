//! Runtime VSOCK endpoint value owned by one sandbox connection session.

use std::error::Error;
use std::fmt;

const VSOCK_ANY: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxEndpoint {
    host_cid: u32,
    port: u32,
}

impl SandboxEndpoint {
    pub fn new(host_cid: u32, port: u32) -> Result<Self, SandboxEndpointError> {
        if host_cid == VSOCK_ANY {
            return Err(SandboxEndpointError::AnyHostCid);
        }
        if port == VSOCK_ANY {
            return Err(SandboxEndpointError::AnyPort);
        }
        Ok(Self { host_cid, port })
    }

    pub const fn host_cid(self) -> u32 {
        self.host_cid
    }

    pub const fn port(self) -> u32 {
        self.port
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEndpointError {
    AnyHostCid,
    AnyPort,
}

impl fmt::Display for SandboxEndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AnyHostCid => formatter.write_str("sandbox VSOCK host CID must be concrete"),
            Self::AnyPort => formatter.write_str("sandbox VSOCK port must be concrete"),
        }
    }
}

impl Error for SandboxEndpointError {}
