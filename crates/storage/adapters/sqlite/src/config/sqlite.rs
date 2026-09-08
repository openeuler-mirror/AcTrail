//! SQLite storage configuration parsing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::semantic_actions::storage_meta::ColdFieldCompression;

pub const SQLITE_STORAGE_CONFIG_PREFIX: &str = "storage_sqlite_";
pub const SQLITE_DEFAULT_BUSY_TIMEOUT_MS: u64 = 5000;
pub const SQLITE_DEFAULT_EVENT_PAYLOAD_DICTIONARY_CACHE_BYTES: usize = 32 * 1024 * 1024;
pub const SQLITE_DEFAULT_EVENT_PATH_DICTIONARY_CACHE_BYTES: usize = 8 * 1024 * 1024;
pub const SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_EVENTS: usize = 256;
pub const SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_UNCOMPRESSED_BYTES: usize = 1024 * 1024;
pub const SQLITE_DEFAULT_EVENT_RECORD_BLOCK_ZSTD_LEVEL: i32 = 3;
pub const SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS: usize = 65_536;
pub const SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventRecordLayout {
    Rows,
    Blocks,
}

impl EventRecordLayout {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rows => "rows",
            Self::Blocks => "blocks",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "rows" => Ok(Self::Rows),
            "blocks" => Ok(Self::Blocks),
            _ => Err("expected rows or blocks".to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteStorageConfig {
    pub path: PathBuf,
    pub busy_timeout_ms: u64,
    pub cold_field_compression_min_bytes: usize,
    pub cold_field_zstd_level: i32,
    pub event_payload_dictionary_cache_bytes: usize,
    pub event_path_dictionary_cache_bytes: usize,
    pub event_record_layout: EventRecordLayout,
    pub event_record_block_max_events: usize,
    pub event_record_block_max_uncompressed_bytes: usize,
    pub event_record_block_zstd_level: i32,
}

impl SqliteStorageConfig {
    pub fn cold_field_compression(&self) -> ColdFieldCompression {
        ColdFieldCompression {
            zstd_level: self.cold_field_zstd_level,
            compression_min_bytes: self.cold_field_compression_min_bytes,
        }
    }

    pub fn parse_entries(
        entries: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, String> {
        let values = ConfigValues::new(entries)?;
        let default_compression = ColdFieldCompression::DEFAULT;
        Ok(Self {
            path: PathBuf::from(values.required("path")?),
            busy_timeout_ms: values.required_positive_u64("busy_timeout_ms")?,
            cold_field_compression_min_bytes: values
                .optional_usize("cold_field_compression_min_bytes")?
                .unwrap_or(default_compression.compression_min_bytes),
            cold_field_zstd_level: values
                .optional_i32("cold_field_zstd_level")?
                .unwrap_or(default_compression.zstd_level),
            event_payload_dictionary_cache_bytes: values
                .optional_usize("event_payload_dictionary_cache_bytes")?
                .unwrap_or(SQLITE_DEFAULT_EVENT_PAYLOAD_DICTIONARY_CACHE_BYTES),
            event_path_dictionary_cache_bytes: values
                .optional_usize("event_path_dictionary_cache_bytes")?
                .unwrap_or(SQLITE_DEFAULT_EVENT_PATH_DICTIONARY_CACHE_BYTES),
            event_record_layout: values
                .optional("event_record_layout")
                .map(EventRecordLayout::parse)
                .transpose()?
                .unwrap_or(EventRecordLayout::Rows),
            event_record_block_max_events: values
                .optional_bounded_positive_usize(
                    "event_record_block_max_events",
                    SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS,
                )?
                .unwrap_or(SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_EVENTS),
            event_record_block_max_uncompressed_bytes: values
                .optional_bounded_positive_usize(
                    "event_record_block_max_uncompressed_bytes",
                    SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES,
                )?
                .unwrap_or(SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_UNCOMPRESSED_BYTES),
            event_record_block_zstd_level: values
                .optional_i32("event_record_block_zstd_level")?
                .map(validate_zstd_level)
                .transpose()?
                .unwrap_or(SQLITE_DEFAULT_EVENT_RECORD_BLOCK_ZSTD_LEVEL),
        })
    }

    pub fn direct_path(path: impl AsRef<Path>) -> Self {
        let default_compression = ColdFieldCompression::DEFAULT;
        Self {
            path: path.as_ref().to_path_buf(),
            busy_timeout_ms: SQLITE_DEFAULT_BUSY_TIMEOUT_MS,
            cold_field_compression_min_bytes: default_compression.compression_min_bytes,
            cold_field_zstd_level: default_compression.zstd_level,
            event_payload_dictionary_cache_bytes:
                SQLITE_DEFAULT_EVENT_PAYLOAD_DICTIONARY_CACHE_BYTES,
            event_path_dictionary_cache_bytes: SQLITE_DEFAULT_EVENT_PATH_DICTIONARY_CACHE_BYTES,
            event_record_layout: EventRecordLayout::Rows,
            event_record_block_max_events: SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_EVENTS,
            event_record_block_max_uncompressed_bytes:
                SQLITE_DEFAULT_EVENT_RECORD_BLOCK_MAX_UNCOMPRESSED_BYTES,
            event_record_block_zstd_level: SQLITE_DEFAULT_EVENT_RECORD_BLOCK_ZSTD_LEVEL,
        }
    }
}

struct ConfigValues {
    values: BTreeMap<String, String>,
}

impl ConfigValues {
    fn new(entries: impl IntoIterator<Item = (String, String)>) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        for (key, value) in entries {
            reject_unknown_key(&key)?;
            if values.insert(key.clone(), value).is_some() {
                return Err(format!(
                    "duplicate config key {SQLITE_STORAGE_CONFIG_PREFIX}{key}"
                ));
            }
        }
        Ok(Self { values })
    }

    fn required(&self, key: &'static str) -> Result<String, String> {
        self.values
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {SQLITE_STORAGE_CONFIG_PREFIX}{key}"))
    }

    fn optional(&self, key: &'static str) -> Option<&str> {
        self.values
            .get(key)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
    }

    fn required_positive_u64(&self, key: &'static str) -> Result<u64, String> {
        let raw = self.required(key)?;
        let value = raw
            .parse::<u64>()
            .map_err(|error| format!("invalid {SQLITE_STORAGE_CONFIG_PREFIX}{key}: {error}"))?;
        if value == u64::default() {
            return Err(format!(
                "invalid {SQLITE_STORAGE_CONFIG_PREFIX}{key}: value must be positive"
            ));
        }
        Ok(value)
    }

    fn optional_usize(&self, key: &'static str) -> Result<Option<usize>, String> {
        self.values
            .get(key)
            .filter(|value| !value.is_empty())
            .map(|raw| {
                raw.parse::<usize>().map_err(|error| {
                    format!("invalid {SQLITE_STORAGE_CONFIG_PREFIX}{key}: {error}")
                })
            })
            .transpose()
    }

    fn optional_positive_usize(&self, key: &'static str) -> Result<Option<usize>, String> {
        let value = self.optional_usize(key)?;
        if value == Some(0) {
            return Err(format!(
                "invalid {SQLITE_STORAGE_CONFIG_PREFIX}{key}: value must be positive"
            ));
        }
        Ok(value)
    }

    fn optional_bounded_positive_usize(
        &self,
        key: &'static str,
        maximum: usize,
    ) -> Result<Option<usize>, String> {
        let value = self.optional_positive_usize(key)?;
        if value.is_some_and(|value| value > maximum) {
            return Err(format!(
                "invalid {SQLITE_STORAGE_CONFIG_PREFIX}{key}: value must not exceed {maximum}"
            ));
        }
        Ok(value)
    }

    fn optional_i32(&self, key: &'static str) -> Result<Option<i32>, String> {
        self.values
            .get(key)
            .filter(|value| !value.is_empty())
            .map(|raw| {
                raw.parse::<i32>().map_err(|error| {
                    format!("invalid {SQLITE_STORAGE_CONFIG_PREFIX}{key}: {error}")
                })
            })
            .transpose()
    }
}

fn reject_unknown_key(key: &str) -> Result<(), String> {
    match key {
        "path"
        | "busy_timeout_ms"
        | "cold_field_compression_min_bytes"
        | "cold_field_zstd_level"
        | "event_payload_dictionary_cache_bytes"
        | "event_path_dictionary_cache_bytes"
        | "event_record_layout"
        | "event_record_block_max_events"
        | "event_record_block_max_uncompressed_bytes"
        | "event_record_block_zstd_level" => Ok(()),
        _ => Err(format!(
            "unknown config key {SQLITE_STORAGE_CONFIG_PREFIX}{key}"
        )),
    }
}

fn validate_zstd_level(level: i32) -> Result<i32, String> {
    if (-7..=22).contains(&level) {
        Ok(level)
    } else {
        Err(format!(
            "invalid {SQLITE_STORAGE_CONFIG_PREFIX}event_record_block_zstd_level: expected -7..=22"
        ))
    }
}
