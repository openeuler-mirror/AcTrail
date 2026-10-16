use super::ResourceRetentionConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceStorageSqliteConfig {
    pub path: PathBuf,
    pub write_timeout_ms: u64,
    pub retention: ResourceRetentionConfig,
}
impl Default for ResourceStorageSqliteConfig {
    fn default() -> Self {
        Self {
            path: "./actrail-resources.sqlite".into(),
            write_timeout_ms: 1000,
            retention: ResourceRetentionConfig::default(),
        }
    }
}
impl ResourceStorageSqliteConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let config: Self = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource SQLite: {e}"))?;
        if config.path.as_os_str().is_empty() {
            return Err("resource SQLite path must not be empty".into());
        }
        if config.write_timeout_ms == 0 {
            return Err("resource SQLite write_timeout_ms must be positive".into());
        }
        config.retention.validate()?;
        Ok(config)
    }
    pub fn local_files(&self) -> Vec<PathBuf> {
        ["", "-wal", "-shm"]
            .into_iter()
            .map(|suffix| {
                let mut path = self.path.as_os_str().to_os_string();
                path.push(suffix);
                PathBuf::from(path)
            })
            .collect()
    }
    pub fn resolve_paths(&mut self, base: &Path) {
        if self.path.is_relative() {
            self.path = base.join(&self.path);
        }
    }
}
