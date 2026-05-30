//! Sensor grouping by observation surface.

pub(crate) mod file;
pub(crate) mod ipc;
pub(crate) mod network;
pub(crate) mod payload;
pub(crate) mod process;
pub(crate) mod stdio;

pub fn potential_descriptors() -> Vec<model_core::capability::CapabilityDescriptor> {
    let mut descriptors = Vec::new();
    descriptors.extend(process::descriptors());
    descriptors.extend(file::descriptors());
    descriptors.extend(network::descriptors());
    descriptors.extend(payload::descriptors());
    descriptors.extend(ipc::descriptors());
    descriptors.extend(stdio::descriptors());
    descriptors
}
