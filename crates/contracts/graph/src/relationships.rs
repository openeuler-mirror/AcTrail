//! Graph relationship contracts between nodes.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphRelationship {
    RootOwns,
    ProcessSpawned,
    ProcessObserved,
    ProcessEmittedPayload,
    ProcessAccessedFile,
    ProcessOpenedChannel,
    TraceHasDiagnostic,
}
