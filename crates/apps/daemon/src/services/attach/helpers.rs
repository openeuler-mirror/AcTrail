//! Attach-service helpers that do not touch storage or collectors.

use std::collections::BTreeSet;

use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use provider_evidence::EvidenceBundle;
use provider_label::{ProviderClassifier, ProviderLabelRecord};
use trace_runtime::sensor_plan::SensorPlan;

pub(super) fn collector_capability_requests(
    profile_requests: &[CapabilityRequest],
    sensor_plan: &SensorPlan,
    collector_name: &str,
) -> Vec<CapabilityRequest> {
    let assigned = sensor_plan
        .collectors
        .iter()
        .find(|plan| plan.collector_name.as_str() == collector_name)
        .map(|plan| {
            plan.capabilities
                .iter()
                .cloned()
                .collect::<BTreeSet<Capability>>()
        })
        .unwrap_or_default();
    profile_requests
        .iter()
        .filter(|request| assigned.contains(&request.capability))
        .cloned()
        .collect()
}

pub(super) fn capability_requested(
    requests: &[CapabilityRequest],
    capability: &Capability,
) -> bool {
    requests
        .iter()
        .any(|request| request.mode != RequestMode::Disabled && request.capability == *capability)
}

pub(super) struct NoopProviderClassifier;

impl ProviderClassifier for NoopProviderClassifier {
    fn classify(&self, _evidence: &EvidenceBundle) -> ProviderLabelRecord {
        ProviderLabelRecord::unknown(String::new())
    }
}
