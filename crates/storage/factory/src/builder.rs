use std::fs;
use std::path::Path;
use std::time::Duration;

use sqlite_storage::SqliteStorage;
use storage_core::{StorageBackend, StorageError, StorageOpenMode};

use crate::StorageConfig;

pub fn open_storage_backend(
    config: &StorageConfig,
    mode: StorageOpenMode,
) -> Result<Box<dyn StorageBackend>, StorageError> {
    match (config, mode) {
        (StorageConfig::Sqlite(config), StorageOpenMode::ReadWrite) => {
            create_parent_directory(&config.path)?;
            SqliteStorage::open_with_busy_timeout(
                &config.path,
                Duration::from_millis(config.busy_timeout_ms),
            )
            .map(|storage| Box::new(storage) as Box<dyn StorageBackend>)
            .map_err(|error| StorageError::new("open_sqlite_storage", error.to_string()))
        }
        (StorageConfig::Sqlite(config), StorageOpenMode::ReadOnly) => {
            SqliteStorage::open_read_only(&config.path)
                .map(|storage| Box::new(storage) as Box<dyn StorageBackend>)
                .map_err(|error| {
                    StorageError::new("open_sqlite_storage_read_only", error.to_string())
                })
        }
    }
}

fn create_parent_directory(path: &Path) -> Result<(), StorageError> {
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| {
        StorageError::new(
            "create_sqlite_storage_directory",
            format!(
                "create sqlite storage directory {}: {error}",
                parent.display()
            ),
        )
    })
}
