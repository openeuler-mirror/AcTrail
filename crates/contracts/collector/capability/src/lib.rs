//! Collector capability and guarantee contracts.

use model_core::capability::{CapabilityDescriptor, CapabilitySet};
use model_core::ids::CollectorName;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollectorDescriptor {
    pub name: CollectorName,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub supports_attach_coverage_guard: bool,
    pub supports_existing_pid_attach: bool,
}

impl CollectorDescriptor {
    pub fn capability_set(&self) -> CapabilitySet {
        CapabilitySet::new(
            self.capabilities
                .iter()
                .map(|descriptor| descriptor.capability.clone()),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityBindingFailure {
    pub collector: CollectorName,
    pub detail: String,
}
