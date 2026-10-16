use super::super::sqlite::ResourceStorageSqliteConfig;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseStorageBackendKind {
    #[default]
    Sqlite,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DatabaseStorageConfig {
    pub backend: DatabaseStorageBackendKind,
    pub sqlite: ResourceStorageSqliteConfig,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    backend: DatabaseStorageBackendKind,
    sqlite: toml::Table,
}
impl DatabaseStorageConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource database: {e}"))?;
        Ok(Self {
            backend: raw.backend,
            sqlite: ResourceStorageSqliteConfig::parse(&raw.sqlite)?,
        })
    }
    pub fn local_files(&self) -> Vec<std::path::PathBuf> {
        match self.backend {
            DatabaseStorageBackendKind::Sqlite => self.sqlite.local_files(),
        }
    }
    pub fn resolve_paths(&mut self, base: &std::path::Path) {
        self.sqlite.resolve_paths(base);
    }
}
