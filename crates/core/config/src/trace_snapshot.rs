//! Immutable trace-time snapshots derived from mutable config.

use std::time::SystemTime;

use model_core::capability::CapabilityRequest;
use model_core::ids::ProfileName;

use crate::capture_profile::CaptureProfile;
use crate::export::ExportConfig;
use crate::policy::PolicyConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureProfileSnapshot {
    pub profile_name: ProfileName,
    pub captured_at: SystemTime,
    pub capability_requests: Vec<CapabilityRequest>,
    pub classify_providers: bool,
    pub enable_payload_collectors: bool,
}

impl CaptureProfileSnapshot {
    pub fn from_profile(profile: &CaptureProfile, captured_at: SystemTime) -> Self {
        Self {
            profile_name: profile.name.clone(),
            captured_at,
            capability_requests: profile.capabilities.clone(),
            classify_providers: profile.classify_providers,
            enable_payload_collectors: profile.enable_payload_collectors,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceExecutionSnapshot {
    pub capture_profile: CaptureProfileSnapshot,
    pub policy: PolicyConfig,
    pub export: ExportConfig,
}
