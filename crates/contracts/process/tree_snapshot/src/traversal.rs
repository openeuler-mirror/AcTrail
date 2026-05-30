//! Traversal contracts for deterministic snapshot processing.

use super::snapshot::{ProcessSnapshot, TreeSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotTraversalOrder {
    RootFirst,
    ParentBeforeChild,
}

pub trait SnapshotTraversal {
    fn ordered<'a>(
        &'a self,
        snapshot: &'a TreeSnapshot,
        order: SnapshotTraversalOrder,
    ) -> Vec<&'a ProcessSnapshot>;
}
