//! Capture capability advertised by the network-control service.

use collector_capability::CollectorDescriptor;
use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};
use model_core::ids::CollectorName;

use super::audit::NETWORK_CONTROL_COLLECTOR_NAME;
use super::service::NetworkControlService;

impl NetworkControlService {
    pub(crate) fn descriptor() -> CollectorDescriptor {
        CollectorDescriptor {
            name: CollectorName::new(NETWORK_CONTROL_COLLECTOR_NAME),
            capabilities: vec![CapabilityDescriptor::new(
                Capability::EnforcementNetworkConnectSeccomp,
                vec![
                    CapabilityField::new(
                        "remote_endpoint",
                        GuaranteeClass::GuaranteedByTransportCollector,
                    ),
                    CapabilityField::new(
                        "decision",
                        GuaranteeClass::GuaranteedByTransportCollector,
                    ),
                    CapabilityField::new(
                        "decision_source",
                        GuaranteeClass::GuaranteedByTransportCollector,
                    ),
                ],
            )],
            supports_attach_coverage_guard: false,
            supports_existing_pid_attach: false,
        }
    }
}
