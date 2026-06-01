//! Local IPC and Unix-socket observation sensor skeleton.

use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};

pub fn descriptors() -> Vec<CapabilityDescriptor> {
    vec![
        CapabilityDescriptor::new(
            Capability::IpcUnixSocket,
            vec![CapabilityField::new(
                "unix_socket_peer",
                GuaranteeClass::AvailableWhenMetadataObservable,
            )],
        ),
        CapabilityDescriptor::new(
            Capability::IpcPipeFifo,
            vec![CapabilityField::new(
                "pipe_fifo_flow",
                GuaranteeClass::GuaranteedByTransportCollector,
            )],
        ),
    ]
}
