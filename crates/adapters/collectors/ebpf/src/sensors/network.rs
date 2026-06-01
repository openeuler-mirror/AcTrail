//! Network transport and DNS observation sensor skeleton.

use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};

pub fn descriptors() -> Vec<CapabilityDescriptor> {
    vec![
        CapabilityDescriptor::new(
            Capability::NetTransport,
            vec![CapabilityField::new(
                "endpoint_direction",
                GuaranteeClass::GuaranteedByTransportCollector,
            )],
        ),
        CapabilityDescriptor::new(
            Capability::NetDns,
            vec![CapabilityField::new(
                "dns_message",
                GuaranteeClass::AvailableWhenMetadataObservable,
            )],
        ),
        CapabilityDescriptor::new(
            Capability::NetTlsMetadata,
            vec![CapabilityField::new(
                "sni_alpn",
                GuaranteeClass::AvailableWhenMetadataObservable,
            )],
        ),
    ]
}
