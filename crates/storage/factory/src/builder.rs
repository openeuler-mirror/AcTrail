//! Open the selected storage implementation with the shared configuration.

use std::time::Duration;

use sqlite_storage::SqliteStorage;
use storage_core::{StorageBackend, StorageError, StorageOpenMode};

use storage_contracts::StorageConfig;

pub fn open_storage_backend(
    config: &StorageConfig,
    mode: StorageOpenMode,
) -> Result<Box<dyn StorageBackend>, StorageError> {
    match (config, mode) {
        (StorageConfig::NoOp, _) => Ok(Box::new(storage_noop::NoOpStorage::new())),
        (StorageConfig::Sqlite(config), StorageOpenMode::ReadWrite) => {
            SqliteStorage::open_with_options(
                &config.path,
                Some(Duration::from_millis(config.busy_timeout_ms)),
                sqlite_storage::ColdFieldCompression::from(config),
                config.event_payload_dictionary_cache_bytes,
                config.event_path_dictionary_cache_bytes,
                config.event_record_layout,
                config.event_record_block_max_events,
                config.event_record_block_max_uncompressed_bytes,
                config.event_record_block_zstd_level,
            )
            .map(|storage| {
                let storage = match config.payload_retention_limits {
                    Some(limits) => storage.with_payload_retention_limits(
                        limits,
                        config.payload_retention_max_cached_traces,
                    ),
                    None => storage,
                };
                Box::new(storage) as Box<dyn StorageBackend>
            })
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
