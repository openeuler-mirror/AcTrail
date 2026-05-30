//! Stdio payload observation sensor declaration.

use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};

pub fn descriptors() -> Vec<CapabilityDescriptor> {
    vec![CapabilityDescriptor::new(
        Capability::StdioChunk,
        vec![CapabilityField::new(
            "stdin_stdout_stderr_segment",
            GuaranteeClass::RequiresPayloadCollector,
        )],
    )]
}
