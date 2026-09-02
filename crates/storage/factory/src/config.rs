use std::path::Path;
use std::str::FromStr;

use sqlite_storage::{EventRecordLayout, SqliteStorageConfig};

use crate::parser::parse_storage_config;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageBackendKind {
    Sqlite,
}

impl StorageBackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
        }
    }
}

impl FromStr for StorageBackendKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sqlite" => Ok(Self::Sqlite),
            _ => Err("expected sqlite".to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageConfig {
    Sqlite(SqliteStorageConfig),
}

impl StorageConfig {
    pub fn parse(raw: &str) -> Result<Self, String> {
        parse_storage_config(raw)
    }

    pub fn sqlite_path(path: impl AsRef<Path>) -> Self {
        Self::Sqlite(SqliteStorageConfig::direct_path(path))
    }

    pub fn sqlite(path: impl AsRef<Path>, busy_timeout_ms: u64) -> Self {
        let mut config = SqliteStorageConfig::direct_path(path);
        config.busy_timeout_ms = busy_timeout_ms;
        Self::Sqlite(config)
    }

    pub fn sqlite_with_compression(
        path: impl AsRef<Path>,
        busy_timeout_ms: u64,
        cold_field_compression_min_bytes: usize,
        cold_field_zstd_level: i32,
    ) -> Self {
        Self::sqlite_with_options(
            path,
            busy_timeout_ms,
            cold_field_compression_min_bytes,
            cold_field_zstd_level,
            sqlite_storage::SQLITE_DEFAULT_EVENT_PAYLOAD_DICTIONARY_CACHE_BYTES,
            sqlite_storage::SQLITE_DEFAULT_EVENT_PATH_DICTIONARY_CACHE_BYTES,
            EventRecordLayout::Rows,
            sqlite_storage::SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_EVENTS,
            sqlite_storage::SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_UNCOMPRESSED_BYTES,
            sqlite_storage::SQLITE_DEFAULT_EVENT_RECORD_BLOCK_ZSTD_LEVEL,
        )
    }

    pub fn sqlite_with_options(
        path: impl AsRef<Path>,
        busy_timeout_ms: u64,
        cold_field_compression_min_bytes: usize,
        cold_field_zstd_level: i32,
        event_payload_dictionary_cache_bytes: usize,
        event_path_dictionary_cache_bytes: usize,
        event_record_layout: EventRecordLayout,
        event_record_block_max_events: usize,
        event_record_block_max_uncompressed_bytes: usize,
        event_record_block_zstd_level: i32,
    ) -> Self {
        Self::Sqlite(SqliteStorageConfig {
            path: path.as_ref().to_path_buf(),
            busy_timeout_ms,
            cold_field_compression_min_bytes,
            cold_field_zstd_level,
            event_payload_dictionary_cache_bytes,
            event_path_dictionary_cache_bytes,
            event_record_layout,
            event_record_block_max_events,
            event_record_block_max_uncompressed_bytes,
            event_record_block_zstd_level,
        })
    }

    pub const fn backend(&self) -> StorageBackendKind {
        match self {
            Self::Sqlite(_) => StorageBackendKind::Sqlite,
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Sqlite(config) => &config.path,
        }
    }

    pub const fn sqlite_busy_timeout_ms(&self) -> u64 {
        match self {
            Self::Sqlite(config) => config.busy_timeout_ms,
        }
    }

    pub const fn sqlite_cold_field_compression_min_bytes(&self) -> usize {
        match self {
            Self::Sqlite(config) => config.cold_field_compression_min_bytes,
        }
    }

    pub const fn sqlite_cold_field_zstd_level(&self) -> i32 {
        match self {
            Self::Sqlite(config) => config.cold_field_zstd_level,
        }
    }

    pub const fn sqlite_event_payload_dictionary_cache_bytes(&self) -> usize {
        match self {
            Self::Sqlite(config) => config.event_payload_dictionary_cache_bytes,
        }
    }

    pub const fn sqlite_event_path_dictionary_cache_bytes(&self) -> usize {
        match self {
            Self::Sqlite(config) => config.event_path_dictionary_cache_bytes,
        }
    }

    pub const fn sqlite_event_record_layout(&self) -> EventRecordLayout {
        match self {
            Self::Sqlite(config) => config.event_record_layout,
        }
    }

    pub const fn sqlite_event_record_block_max_events(&self) -> usize {
        match self {
            Self::Sqlite(config) => config.event_record_block_max_events,
        }
    }

    pub const fn sqlite_event_record_block_max_uncompressed_bytes(&self) -> usize {
        match self {
            Self::Sqlite(config) => config.event_record_block_max_uncompressed_bytes,
        }
    }

    pub const fn sqlite_event_record_block_zstd_level(&self) -> i32 {
        match self {
            Self::Sqlite(config) => config.event_record_block_zstd_level,
        }
    }
}
