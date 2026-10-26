use std::path::Path;
use std::str::FromStr;

use sqlite_storage::{EventRecordLayout, SqliteStorageConfig};

use crate::parser::parse_storage_config;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageBackendKind {
    Sqlite,
    NoOp,
}

impl StorageBackendKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::NoOp => "noop",
        }
    }
}

impl FromStr for StorageBackendKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "sqlite" => Ok(Self::Sqlite),
            "noop" => Ok(Self::NoOp),
            _ => Err("expected sqlite or noop".to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageConfig {
    Sqlite(SqliteStorageConfig),
    NoOp,
}

impl StorageConfig {
    /// Configure write-side limits. The cache capacity is only a performance
    /// hint; zero disables caching without disabling retention limits.
    pub fn with_payload_retention_limits(
        mut self,
        limits: storage_core::PayloadRetentionLimits,
        max_cached_traces: usize,
    ) -> Self {
        match &mut self {
            Self::Sqlite(config) => {
                config.payload_retention_limits = Some(limits);
                config.payload_retention_max_cached_traces = max_cached_traces;
            }
            Self::NoOp => {}
        }
        self
    }

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
            payload_retention_limits: None,
            payload_retention_max_cached_traces: 0,
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
            Self::NoOp => StorageBackendKind::NoOp,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.sqlite_config().map(|config| config.path.as_path())
    }

    pub const fn sqlite_config(&self) -> Option<&SqliteStorageConfig> {
        match self {
            Self::Sqlite(config) => Some(config),
            Self::NoOp => None,
        }
    }
}
