//! TLS plaintext payload observation sensor declaration.

use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};

pub fn descriptors() -> Vec<CapabilityDescriptor> {
    vec![
        CapabilityDescriptor::new(
            Capability::TlsPlaintextPayload,
            vec![CapabilityField::new(
                "tls_plaintext_segment",
                GuaranteeClass::RequiresPayloadCollector,
            )],
        ),
        CapabilityDescriptor::new(
            Capability::SocketPlaintextPayload,
            vec![CapabilityField::new(
                "socket_plaintext_segment",
                GuaranteeClass::RequiresPayloadCollector,
            )],
        ),
    ]
}
