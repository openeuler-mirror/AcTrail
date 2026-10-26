use super::ResourceQuery;
use crate::{
    model::ResourceSample,
    storage::{ResourceStorageBackend, ResourceStorageError},
};
pub trait DatabaseStorageBackend: ResourceStorageBackend {
    fn query(&self, query: &ResourceQuery) -> Result<Vec<ResourceSample>, ResourceStorageError>;
    fn delete(&mut self, query: &ResourceQuery) -> Result<usize, ResourceStorageError>;
}
