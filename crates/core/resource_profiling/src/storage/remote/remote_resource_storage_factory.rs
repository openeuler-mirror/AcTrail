use super::{RemoteResourceStorageBackend, RemoteResourceStorageConfig};
use crate::storage::ResourceStorageError;
pub struct RemoteResourceStorageFactory;
impl RemoteResourceStorageFactory {
    pub fn build(
        config: RemoteResourceStorageConfig,
    ) -> Result<RemoteResourceStorageBackend, ResourceStorageError> {
        RemoteResourceStorageBackend::new(config)
    }
}
