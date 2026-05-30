//! File-access observation sensor skeleton.

use model_core::capability::{Capability, CapabilityDescriptor, CapabilityField, GuaranteeClass};

pub fn descriptors() -> Vec<CapabilityDescriptor> {
    vec![
        CapabilityDescriptor::new(
            Capability::FsAccessBasic,
            vec![
                CapabilityField::new(
                    "path_op_errno",
                    GuaranteeClass::AvailableWhenMetadataObservable,
                ),
                CapabilityField::new(
                    "file_path_mutation_syscalls",
                    GuaranteeClass::GuaranteedByTransportCollector,
                ),
            ],
        ),
        CapabilityDescriptor::new(
            Capability::FsMmap,
            vec![CapabilityField::new(
                "mmap_shared_file_access",
                GuaranteeClass::GuaranteedByTransportCollector,
            )],
        ),
        CapabilityDescriptor::new(
            Capability::FsExecAccess,
            vec![CapabilityField::new(
                "exec_file_access",
                GuaranteeClass::GuaranteedByTransportCollector,
            )],
        ),
    ]
}
