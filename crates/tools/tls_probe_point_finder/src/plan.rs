//! Structured probe-point plan for TLS payload capture runtimes.

use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbePointPlan {
    pub target: TargetIdentity,
    pub provider: TlsProvider,
    pub source: ProbeSource,
    pub resolver: String,
    pub binary: ProbeBinary,
    pub points: Vec<ProbePoint>,
}

impl ProbePointPlan {
    pub fn has_payload_closure(&self) -> bool {
        let inbound = self
            .points
            .iter()
            .any(|point| point.direction == PayloadDirection::Inbound);
        let outbound = self
            .points
            .iter()
            .any(|point| point.direction == PayloadDirection::Outbound);
        inbound && outbound
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetIdentity {
    pub binary: PathBuf,
    pub architecture: String,
    pub build_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeBinary {
    pub path: PathBuf,
    pub architecture: String,
    pub build_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbePoint {
    pub symbol: String,
    pub direction: PayloadDirection,
    pub attach: AttachPoint,
    pub capture: CaptureStrategy,
    pub virtual_address: u64,
    pub file_offset: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TlsProvider {
    OpenSsl,
    BoringSsl,
    Rustls,
}

impl TlsProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenSsl => "openssl",
            Self::BoringSsl => "boringssl",
            Self::Rustls => "rustls",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeSource {
    Executable,
    SharedLibrary,
}

impl ProbeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Executable => "executable",
            Self::SharedLibrary => "shared-library",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadDirection {
    Inbound,
    Outbound,
    Control,
}

impl PayloadDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
            Self::Control => "control",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachPoint {
    Entry,
    Return,
}

impl AttachPoint {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::Return => "return",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureStrategy {
    EntryBuffer,
    ReturnBufferFromEntryState,
}

impl CaptureStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EntryBuffer => "entry-buffer",
            Self::ReturnBufferFromEntryState => "return-buffer-from-entry-state",
        }
    }
}
