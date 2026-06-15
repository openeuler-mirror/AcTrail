//! SQLite storage configuration parsing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SQLITE_STORAGE_CONFIG_PREFIX: &str = "storage_sqlite_";
pub const SQLITE_DEFAULT_BUSY_TIMEOUT_MS: u64 = 5000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteStorageConfig {
    pub path: PathBuf,
    pub busy_timeout_ms: u64,
}

impl SqliteStorageConfig {
    pub fn parse_entries(
        entries: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, String> {
        let values = ConfigValues::new(entries)?;
        Ok(Self {
            path: PathBuf::from(values.required("path")?),
            busy_timeout_ms: values.required_positive_u64("busy_timeout_ms")?,
        })
    }

    pub fn direct_path(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            busy_timeout_ms: SQLITE_DEFAULT_BUSY_TIMEOUT_MS,
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
}

fn reject_unknown_key(key: &str) -> Result<(), String> {
    match key {
        "path" | "busy_timeout_ms" => Ok(()),
        _ => Err(format!(
            "unknown config key {SQLITE_STORAGE_CONFIG_PREFIX}{key}"
        )),
    }
}
