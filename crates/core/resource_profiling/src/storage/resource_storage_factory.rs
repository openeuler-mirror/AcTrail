use super::{
    ResourceStorageBackend, ResourceStorageBackendKind, ResourceStorageConfig,
    ResourceStorageError, database::DatabaseStorageFactory, hybrid::HybridResourceStorageFactory,
    remote::RemoteResourceStorageFactory,
};
pub struct ResourceStorageFactory;
impl ResourceStorageFactory {
    pub fn build(
        config: ResourceStorageConfig,
    ) -> Result<Box<dyn ResourceStorageBackend>, ResourceStorageError> {
        let kind = config.backend;
        let capacity = config.queue_capacity_batches as usize;
        let backend: Box<dyn ResourceStorageBackend> = match kind {
            ResourceStorageBackendKind::Database => DatabaseStorageFactory::build(
                config
                    .database
                    .ok_or_else(|| ResourceStorageError("missing database config".into()))?,
            )?,
            ResourceStorageBackendKind::Remote => Box::new(RemoteResourceStorageFactory::build(
                config
                    .remote
                    .ok_or_else(|| ResourceStorageError("missing remote config".into()))?,
            )?),
            ResourceStorageBackendKind::Hybrid => Box::new(HybridResourceStorageFactory::build(
                config
                    .hybrid
                    .ok_or_else(|| ResourceStorageError("missing hybrid config".into()))?,
            )?),
        };
        if kind == ResourceStorageBackendKind::Hybrid {
            Ok(backend)
        } else {
            Ok(Box::new(
                super::queued_resource_storage_backend::QueuedResourceStorageBackend::new(
                    backend, capacity,
                )?,
            ))
        }
    }
    pub fn open_database(
        config: ResourceStorageConfig,
    ) -> Result<Box<dyn super::database::DatabaseStorageBackend>, ResourceStorageError> {
        let database = match config.backend {
            ResourceStorageBackendKind::Database => config
                .database
                .ok_or_else(|| ResourceStorageError("missing database config".into()))?,
            ResourceStorageBackendKind::Remote => {
                return Err(ResourceStorageError(
                    "resource storage is remote; query the receiver".into(),
                ));
            }
            ResourceStorageBackendKind::Hybrid => {
                let mut databases = config
                    .hybrid
                    .ok_or_else(|| ResourceStorageError("missing hybrid config".into()))?
                    .backends
                    .into_iter()
                    .filter_map(|child| child.database);
                let database = databases.next().ok_or_else(|| {
                    ResourceStorageError("no resource database configured".into())
                })?;
                if databases.next().is_some() {
                    return Err(ResourceStorageError(
                        "multiple resource databases; select a database file explicitly".into(),
                    ));
                }
                database
            }
        };
        DatabaseStorageFactory::open_reader(database)
    }
}
