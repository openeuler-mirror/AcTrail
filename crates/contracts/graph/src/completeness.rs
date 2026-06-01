//! Graph completeness and degradation markers.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphCompleteness {
    Complete,
    Snapshot,
    Degraded,
    Purged,
}
