use std::collections::BTreeMap;

use sqlite_storage::{SQLITE_STORAGE_CONFIG_PREFIX, SqliteStorageConfig};

use crate::{StorageBackendKind, StorageConfig};

pub(crate) fn parse_storage_config(raw: &str) -> Result<StorageConfig, String> {
    let values = StorageConfigValues::parse(raw)?;
    let backend = values
        .required("storage_backend")?
        .parse::<StorageBackendKind>()
        .map_err(|error| format!("invalid storage_backend: {error}"))?;
    match backend {
        StorageBackendKind::Sqlite => Ok(StorageConfig::Sqlite(
            SqliteStorageConfig::parse_entries(values.prefixed(SQLITE_STORAGE_CONFIG_PREFIX))?,
        )),
    }
}

struct StorageConfigValues {
    values: BTreeMap<String, String>,
}

impl StorageConfigValues {
    fn parse(raw: &str) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        let mut inside_ignored_section = false;
        for (line_index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(is_ignored_section) = parse_section_header(trimmed, line_index + 1)? {
                inside_ignored_section = is_ignored_section;
                continue;
            }
            if inside_ignored_section {
                continue;
            }
            let (key, value) = trimmed
                .split_once('=')
                .ok_or_else(|| format!("invalid config line {}", line_index + 1))?;
            let key = key.trim();
            reject_legacy_storage_key(key)?;
            if key != "storage_backend" && !key.starts_with(SQLITE_STORAGE_CONFIG_PREFIX) {
                continue;
            }
            let value = unquote(value.trim())?;
            if values.insert(key.to_string(), value).is_some() {
                return Err(format!("duplicate config key {key}"));
            }
        }
        Ok(Self { values })
    }

    fn required(&self, key: &'static str) -> Result<String, String> {
        self.values
            .get(key)
            .cloned()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("missing config key {key}"))
    }

    fn prefixed(&self, prefix: &'static str) -> Vec<(String, String)> {
        self.values
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix(prefix)
                    .map(|stripped| (stripped.to_string(), value.clone()))
            })
            .collect()
    }
}

fn parse_section_header(line: &str, line_number: usize) -> Result<Option<bool>, String> {
    if line.starts_with("[[") {
        if !line.ends_with("]]") {
            return Err(format!("invalid config section line {line_number}"));
        }
        if line == "[[export.routes]]" {
            return Ok(Some(true));
        }
        return Err(format!("unsupported config section line {line_number}"));
    }
    if line.ends_with("]]") {
        return Err(format!("invalid config section line {line_number}"));
    }
    if !(line.starts_with('[') || line.ends_with(']')) {
        return Ok(None);
    }
    if !(line.starts_with('[') && line.ends_with(']')) {
        return Err(format!("invalid config section line {line_number}"));
    }
    let section = &line[1..line.len() - 1];
    if section == "export"
        || section.starts_with("export.routes.")
        || section == "semantic_retention"
        || section.starts_with("semantic_retention.")
    {
        return Ok(Some(true));
    }
    Err(format!("unsupported config section line {line_number}"))
}

fn reject_legacy_storage_key(key: &str) -> Result<(), String> {
    match key {
        "storage_path" => Err(
            "unsupported config key storage_path; use storage_sqlite_path for sqlite storage"
                .to_string(),
        ),
        "storage_busy_timeout_ms" => Err(
            "unsupported config key storage_busy_timeout_ms; use storage_sqlite_busy_timeout_ms"
                .to_string(),
        ),
        _ => Ok(()),
    }
}

fn unquote(value: &str) -> Result<String, String> {
    if value.starts_with('"') || value.ends_with('"') {
        if !(value.starts_with('"') && value.ends_with('"') && value.len() >= 2) {
            return Err(format!("invalid quoted value {value}"));
        }
        return Ok(value[1..value.len() - 1].to_string());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use crate::{StorageBackendKind, StorageConfig};

    #[test]
    fn sqlite_storage_config_parses_backend_section() {
        let config = StorageConfig::parse(
            r#"
            storage_backend = sqlite
            storage_sqlite_path = /tmp/actrail.sqlite
            storage_sqlite_busy_timeout_ms = 5000
            "#,
        )
        .expect("parse sqlite storage config");

        let StorageConfig::Sqlite(sqlite) = config;
        assert_eq!(sqlite.path, std::path::PathBuf::from("/tmp/actrail.sqlite"));
        assert_eq!(sqlite.busy_timeout_ms, 5000);
    }

    #[test]
    fn legacy_storage_path_is_rejected() {
        let error = StorageConfig::parse(
            r#"
            storage_backend = sqlite
            storage_path = /tmp/actrail.sqlite
            storage_sqlite_busy_timeout_ms = 5000
            "#,
        )
        .expect_err("legacy storage key should fail");

        assert!(error.contains("unsupported config key storage_path"));
    }

    #[test]
    fn unknown_storage_backend_is_rejected() {
        let error = StorageConfig::parse(
            r#"
            storage_backend = mysql
            storage_sqlite_path = /tmp/actrail.sqlite
            storage_sqlite_busy_timeout_ms = 5000
            "#,
        )
        .expect_err("unknown storage backend should fail");

        assert!(error.contains("invalid storage_backend"));
        assert_eq!(StorageBackendKind::Sqlite.as_str(), "sqlite");
    }

    #[test]
    fn unknown_backend_specific_key_is_rejected() {
        let error = StorageConfig::parse(
            r#"
            storage_backend = sqlite
            storage_sqlite_path = /tmp/actrail.sqlite
            storage_sqlite_busy_timeout_ms = 5000
            storage_sqlite_unexpected = true
            "#,
        )
        .expect_err("unknown backend-specific storage key should fail");

        assert!(error.contains("unknown config key storage_sqlite_unexpected"));
    }
}
