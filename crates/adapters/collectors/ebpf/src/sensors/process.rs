//! Process lifecycle and exec-context observation sensor skeleton.

use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};

pub fn descriptors() -> Vec<CapabilityDescriptor> {
    vec![
        CapabilityDescriptor::new(
            Capability::ProcLifecycle,
            vec![
                CapabilityField::new(
                    "fork_exit_events",
                    GuaranteeClass::GuaranteedByTransportCollector,
                ),
                CapabilityField::new(
                    "signals_session_process_group",
                    GuaranteeClass::GuaranteedByTransportCollector,
                ),
            ],
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
