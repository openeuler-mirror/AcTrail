use super::{
    HybridChildBackendKind, HybridResourceStorageBackend, HybridResourceStorageConfig,
    child_dispatcher::ChildDispatcher,
};
use crate::storage::{
    ResourceStorageBackend, ResourceStorageError, database::DatabaseStorageFactory,
    remote::RemoteResourceStorageFactory,
};
use std::time::Duration;
pub struct HybridResourceStorageFactory;
impl HybridResourceStorageFactory {
    pub fn build(
        config: HybridResourceStorageConfig,
    ) -> Result<HybridResourceStorageBackend, ResourceStorageError> {
        let mut children = Vec::new();
        for child in config.backends {
            let backend: Box<dyn ResourceStorageBackend> = match child.backend {
                HybridChildBackendKind::Remote => Box::new(RemoteResourceStorageFactory::build(
                    child
                        .remote
                        .ok_or_else(|| ResourceStorageError("missing remote config".into()))?,
                )?),
                HybridChildBackendKind::Database => DatabaseStorageFactory::build(
                    child
                        .database
                        .ok_or_else(|| ResourceStorageError("missing database config".into()))?,
                )?,
            };
            children.push(ChildDispatcher::new(
                child.name,
                backend,
                config.queue_capacity_batches as usize,
                Duration::from_millis(config.write_timeout_ms),
            )?);
        }
        Ok(HybridResourceStorageBackend { children })
    }
}
