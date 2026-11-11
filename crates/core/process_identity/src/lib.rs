//! Process identity contracts, coordinate resolution, and logical-ID management.

mod contract;
mod manager;

pub use contract::{
    HostProcessCoordinates, IdentityLookupError, NamespaceIdentity, NamespaceProcessCoordinates,
    ProcessIdentity, ProcessIdentityReader, ProcessObservation, ProcessRecord,
    ProcessResolutionState,
};
pub use manager::{
    HostProcessLookup, ProcessIdentityError, ProcessIdentityManager, ProcessResolution,
};
