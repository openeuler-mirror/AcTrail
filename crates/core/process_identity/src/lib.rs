//! Process identity contracts, coordinate resolution, and logical-ID management.

mod contract;
mod manager;

pub use contract::{
    HostProcessCoordinates, IdentityLookupError, InitialSuppressedFd, KernelProcessCoordinates,
    NamespaceIdentity, NamespaceProcessCoordinates, ProcessIdentity, ProcessIdentityReader,
    ProcessObservation, ProcessRecord, ProcessResolutionState, ProcessSuppressedFd,
    SuppressedFdPurpose,
};
pub use manager::{ProcessIdentityError, ProcessIdentityManager, ProcessResolution};
