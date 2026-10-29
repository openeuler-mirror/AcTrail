use super::super::{
    database::DatabaseStorageConfig, hybrid::HybridResourceStorageConfig,
    remote::RemoteResourceStorageConfig,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceStorageBackendKind {
    Remote,
    #[default]
    Database,
    Hybrid,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResourceStorageConfig {
    pub backend: ResourceStorageBackendKind,
    pub queue_capacity_batches: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteResourceStorageConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<DatabaseStorageConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<HybridResourceStorageConfig>,
}
impl Default for ResourceStorageConfig {
    fn default() -> Self {
        Self {
            backend: ResourceStorageBackendKind::Database,
            queue_capacity_batches: 64,
            remote: None,
            database: Some(DatabaseStorageConfig::default()),
            hybrid: None,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    backend: ResourceStorageBackendKind,
    queue_capacity_batches: Option<u32>,
    remote: toml::Table,
    database: toml::Table,
    hybrid: toml::Table,
}
impl ResourceStorageConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource_profiling.storage: {e}"))?;
        let mut config = Self {
            backend: raw.backend,
            queue_capacity_batches: raw.queue_capacity_batches.unwrap_or(64),
            remote: None,
            database: None,
            hybrid: None,
        };
        if config.queue_capacity_batches == 0 && raw.backend != ResourceStorageBackendKind::Hybrid {
            return Err("resource storage queue_capacity_batches must be positive".into());
        }
        match raw.backend {
            ResourceStorageBackendKind::Remote => {
                config.remote = Some(RemoteResourceStorageConfig::parse(&raw.remote)?)
            }
            ResourceStorageBackendKind::Database => {
                config.database = Some(DatabaseStorageConfig::parse(&raw.database)?)
            }
            ResourceStorageBackendKind::Hybrid => {
                config.hybrid = Some(HybridResourceStorageConfig::parse(&raw.hybrid)?)
            }
        }
        Ok(config)
    }
    pub fn local_files(&self) -> Vec<std::path::PathBuf> {
        match self.backend {
            ResourceStorageBackendKind::Database => self
                .database
                .as_ref()
                .map(|database| database.local_files())
                .unwrap_or_default(),
            ResourceStorageBackendKind::Hybrid => self
                .hybrid
                .as_ref()
                .map(|hybrid| hybrid.local_files())
                .unwrap_or_default(),
            ResourceStorageBackendKind::Remote => Vec::new(),
        }
    }
    /// Main SQLite files available for local reads, excluding WAL sidecars.
    pub fn local_database_paths(&self) -> Vec<&std::path::Path> {
        match self.backend {
            ResourceStorageBackendKind::Database => self
                .database
                .iter()
                .map(|database| database.sqlite.path.as_path())
                .collect(),
            ResourceStorageBackendKind::Hybrid => self
                .hybrid
                .iter()
                .flat_map(|hybrid| &hybrid.backends)
                .filter_map(|child| {
                    child
                        .database
                        .as_ref()
                        .map(|database| database.sqlite.path.as_path())
                })
                .collect(),
            ResourceStorageBackendKind::Remote => Vec::new(),
        }
    }
    pub fn resolve_paths(&mut self, base: &std::path::Path) {
        if let Some(database) = &mut self.database {
            database.resolve_paths(base);
        }
        if let Some(hybrid) = &mut self.hybrid {
            hybrid.resolve_paths(base);
        }
    }
}
