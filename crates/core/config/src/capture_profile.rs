//! Capture-profile declarations used to build immutable trace snapshots.

use model_core::capability::CapabilityRequest;
use model_core::ids::ProfileName;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureDisposition {
    DegradeTrace,
    FailTrace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureProfile {
    pub name: ProfileName,
    pub capabilities: Vec<CapabilityRequest>,
    pub classify_providers: bool,
    pub enable_payload_collectors: bool,
    pub identity_failure: FailureDisposition,
    pub runtime_loss: FailureDisposition,
}

impl CaptureProfile {
    pub fn new(name: ProfileName, capabilities: Vec<CapabilityRequest>) -> Self {
        Self {
            name,
            capabilities,
            classify_providers: false,
            enable_payload_collectors: false,
            identity_failure: FailureDisposition::DegradeTrace,
            runtime_loss: FailureDisposition::DegradeTrace,
        }
    }
}
