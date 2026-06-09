//! Process lifecycle and exec-context observation sensor skeleton.

use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};

pub fn descriptors() -> Vec<CapabilityDescriptor> {
    vec![
        CapabilityDescriptor::new(
            Capability::ProcLifecycle,
            vec![CapabilityField::new(
                "fork_exec_exit",
                GuaranteeClass::GuaranteedByTransportCollector,
            )],
        ),
        CapabilityDescriptor::new(
            Capability::ProcExecContext,
            vec![CapabilityField::new(
                "exec_context",
                GuaranteeClass::AvailableWhenMetadataObservable,
            )],
        ),
    ]
}
