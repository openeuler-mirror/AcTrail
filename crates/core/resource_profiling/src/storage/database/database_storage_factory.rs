use super::{
    DatabaseStorageBackend, DatabaseStorageBackendKind, DatabaseStorageConfig,
    sqlite::ResourceStorageSqliteBackend,
};
use crate::storage::ResourceStorageError;
pub struct DatabaseStorageFactory;
impl DatabaseStorageFactory {
    pub fn open_reader(
        config: DatabaseStorageConfig,
    ) -> Result<Box<dyn DatabaseStorageBackend>, ResourceStorageError> {
        match config.backend {
            DatabaseStorageBackendKind::Sqlite => Ok(Box::new(
                ResourceStorageSqliteBackend::open_reader(config.sqlite)?,
            )),
        }
    }

    pub fn build(
        config: DatabaseStorageConfig,
    ) -> Result<Box<dyn DatabaseStorageBackend>, ResourceStorageError> {
        match config.backend {
            DatabaseStorageBackendKind::Sqlite => {
                Ok(Box::new(ResourceStorageSqliteBackend::open(config.sqlite)?))
            }
        }
    }
}
