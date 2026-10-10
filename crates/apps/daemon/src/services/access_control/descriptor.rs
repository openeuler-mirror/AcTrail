use collector_capability::CollectorDescriptor;
use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};
use model_core::ids::CollectorName;

pub(in crate::services) struct LsmAccessControl;

impl LsmAccessControl {
    pub(in crate::services) const COLLECTOR_NAME: &str = "lsm-access-control";

    pub(in crate::services) fn descriptor(
        file: bool,
        command: bool,
        network: bool,
    ) -> CollectorDescriptor {
        let capabilities = [
            (file, Capability::EnforcementFilePermissionLsm),
            (command, Capability::EnforcementCommandExecutionLsm),
            (network, Capability::EnforcementNetworkConnectLsm),
        ]
        .into_iter()
        .filter_map(|(enabled, capability)| {
            enabled.then(|| {
                CapabilityDescriptor::new(
                    capability,
                    vec![CapabilityField::new(
                        "decision",
                        GuaranteeClass::GuaranteedByTransportCollector,
                    )],
                )
            })
        })
        .collect();
        CollectorDescriptor {
            name: CollectorName::new(Self::COLLECTOR_NAME),
            capabilities,
            supports_attach_coverage_guard: false,
            supports_existing_pid_attach: false,
        }
    }
}
