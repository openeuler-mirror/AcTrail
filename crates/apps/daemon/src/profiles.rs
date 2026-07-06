//! Daemon-local capture-profile registry.

use std::collections::{BTreeMap, BTreeSet};

use config_core::capture_profile::CaptureProfile;
use model_core::ids::ProfileName;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonProfileRegistry {
    capture_profiles: BTreeMap<ProfileName, CaptureProfile>,
    launch_only_profiles: BTreeSet<ProfileName>,
}

impl DaemonProfileRegistry {
    pub fn new() -> Self {
        Self {
            capture_profiles: BTreeMap::new(),
            launch_only_profiles: BTreeSet::new(),
        }
    }

    pub fn insert_capture_profile(&mut self, profile: CaptureProfile) {
        self.capture_profiles.insert(profile.name.clone(), profile);
    }

    pub fn insert_launch_profile(&mut self, profile: CaptureProfile) {
        self.launch_only_profiles.insert(profile.name.clone());
        self.capture_profiles.insert(profile.name.clone(), profile);
    }

    pub fn capture_profile(&self, name: &ProfileName) -> Option<&CaptureProfile> {
        self.capture_profiles.get(name)
    }

    pub fn is_launch_only_profile(&self, name: &ProfileName) -> bool {
        self.launch_only_profiles.contains(name)
    }
}
