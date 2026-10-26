use std::cell::RefCell;
use std::rc::Rc;

use storage_core::StorageError;

use crate::resource_state::ResourceState;

/// Discards observation history while retaining resource control state in memory.
pub struct NoOpStorage {
    pub(super) next_process_id: u64,
    pub(super) next_definition_id: u64,
    pub(super) resources: Rc<RefCell<ResourceState>>,
}

impl NoOpStorage {
    pub fn new() -> Self {
        Self {
            next_process_id: 1,
            next_definition_id: 1,
            resources: Rc::new(RefCell::new(ResourceState::default())),
        }
    }

    pub(super) fn trace_not_found() -> StorageError {
        StorageError::new("trace_not_found", "trace not found")
    }
}

impl Default for NoOpStorage {
    fn default() -> Self {
        Self::new()
    }
}
