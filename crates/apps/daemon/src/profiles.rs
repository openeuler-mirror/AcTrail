//! Daemon-local capture-profile registry.

use std::collections::BTreeMap;

use config_core::capture_profile::CaptureProfile;
use model_core::ids::ProfileName;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonProfileRegistry {
    capture_profiles: BTreeMap<ProfileName, CaptureProfile>,
}

impl DaemonProfileRegistry {
    pub fn new() -> Self {
        Self {
            capture_profiles: BTreeMap::new(),
        }
    }

    pub fn insert_capture_profile(&mut self, profile: CaptureProfile) {
        self.capture_profiles.insert(profile.name.clone(), profile);
    }

    pub fn capture_profile(&self, name: &ProfileName) -> Option<&CaptureProfile> {
        self.capture_profiles.get(name)
    }
}
