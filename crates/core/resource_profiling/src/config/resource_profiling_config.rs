use crate::{
    ResourceCollectionConfig, alerts::ResourceAlertConfig, storage::ResourceStorageConfig,
};
use serde::Deserialize;
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceProfilingConfig {
    pub enabled: bool,
    pub collection: ResourceCollectionConfig,
    pub storage: ResourceStorageConfig,
    pub alerts: ResourceAlertConfig,
}
impl Default for ResourceProfilingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            collection: Default::default(),
            storage: Default::default(),
            alerts: Default::default(),
        }
    }
}
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    enabled: bool,
    collection: toml::Table,
    storage: toml::Table,
    alerts: toml::Table,
}
impl Default for RawConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            collection: Default::default(),
            storage: Default::default(),
            alerts: Default::default(),
        }
    }
}
impl ResourceProfilingConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource_profiling: {e}"))?;
        if !raw.enabled {
            return Ok(Self::disabled());
        }
        Ok(Self {
            enabled: true,
            collection: ResourceCollectionConfig::parse(&raw.collection)?,
            storage: ResourceStorageConfig::parse(&raw.storage)?,
            alerts: ResourceAlertConfig::parse(&raw.alerts)?,
        })
    }
    pub fn default_document() -> Result<toml::Table, String> {
        let defaults = Self::default();
        let mut table = toml::Table::new();
        table.insert("enabled".into(), true.into());
        table.insert(
            "collection".into(),
            toml::Value::try_from(defaults.collection).map_err(|e| e.to_string())?,
        );
        table.insert(
            "storage".into(),
            toml::Value::try_from(defaults.storage).map_err(|e| e.to_string())?,
        );
        table.insert(
            "alerts".into(),
            toml::Value::Table(ResourceAlertConfig::default_document()?),
        );
        Ok(table)
    }
    pub fn disabled() -> Self {
        let mut c = Self::default();
        c.enabled = false;
        c.collection.host.enabled = false;
        c.collection.workload.enabled = false;
        c
    }
    pub fn workload_enabled(&self) -> bool {
        self.enabled && self.collection.workload.enabled
    }
    pub fn host_enabled(&self) -> bool {
        self.enabled && self.collection.host.enabled
    }
    pub fn local_files(&self) -> Vec<std::path::PathBuf> {
        if self.enabled {
            self.storage.local_files()
        } else {
            Vec::new()
        }
    }
    pub fn resolve_paths(&mut self, base: &std::path::Path) {
        self.storage.resolve_paths(base);
    }
}
