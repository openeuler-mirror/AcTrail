//! SQLite storage configuration parsing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::semantic_actions::storage_meta::ColdFieldCompression;

pub const SQLITE_STORAGE_CONFIG_PREFIX: &str = "storage_sqlite_";
pub const SQLITE_DEFAULT_BUSY_TIMEOUT_MS: u64 = 5000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteStorageConfig {
    pub path: PathBuf,
    pub busy_timeout_ms: u64,
    pub cold_field_compression_min_bytes: usize,
    pub cold_field_zstd_level: i32,
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
        })
    }

    pub fn direct_path(path: impl AsRef<Path>) -> Self {
        let default_compression = ColdFieldCompression::DEFAULT;
        Self {
            path: path.as_ref().to_path_buf(),
            busy_timeout_ms: SQLITE_DEFAULT_BUSY_TIMEOUT_MS,
            cold_field_compression_min_bytes: default_compression.compression_min_bytes,
            cold_field_zstd_level: default_compression.zstd_level,
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
        | "cold_field_zstd_level" => Ok(()),
        _ => Err(format!(
            "unknown config key {SQLITE_STORAGE_CONFIG_PREFIX}{key}"
        )),
    }
}
