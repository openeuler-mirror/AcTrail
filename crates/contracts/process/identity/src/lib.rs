//! Shared process identity data without identity allocation or indexing.

mod model;

pub use model::{
    HostProcessCoordinates, NamespaceIdentity, NamespaceProcessCoordinates, ProcessIdentity,
    ProcessObservation, ProcessRecord, ProcessResolutionState,
};
