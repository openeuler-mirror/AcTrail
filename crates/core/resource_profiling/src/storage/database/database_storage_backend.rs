use super::ResourceQuery;
use crate::{
    model::ResourceSample,
    storage::{ResourceStorageBackend, ResourceStorageError},
};
pub trait DatabaseStorageBackend: ResourceStorageBackend {
    /// Returns the newest `query.limit` matching samples, newest first.
    fn query(&self, query: &ResourceQuery) -> Result<Vec<ResourceSample>, ResourceStorageError>;
    /// Streams matching samples oldest first, at most `query.limit` of them,
    /// and returns how many were visited.
    fn scan_ascending(
        &self,
        query: &ResourceQuery,
        visit: &mut dyn FnMut(ResourceSample),
    ) -> Result<u32, ResourceStorageError>;
    /// Age after which retention deletes samples, when retention is enabled.
    fn retention_max_age(&self) -> Option<std::time::Duration> {
        None
    }
    fn delete(&mut self, query: &ResourceQuery) -> Result<usize, ResourceStorageError>;
}
