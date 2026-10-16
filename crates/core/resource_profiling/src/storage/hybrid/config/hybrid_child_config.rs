use crate::storage::{database::DatabaseStorageConfig, remote::RemoteResourceStorageConfig};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridChildBackendKind {
    Remote,
    Database,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HybridChildConfig {
    pub name: String,
    pub backend: HybridChildBackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteResourceStorageConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<DatabaseStorageConfig>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    name: String,
    backend: HybridChildBackendKind,
    remote: Option<toml::Table>,
    database: Option<toml::Table>,
}
impl HybridChildConfig {
    pub(super) fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource hybrid child: {e}"))?;
        if raw.name.trim().is_empty() {
            return Err("resource hybrid child name must not be empty".into());
        }
        let mut child = Self {
            name: raw.name,
            backend: raw.backend,
            remote: None,
            database: None,
        };
        match raw.backend {
            HybridChildBackendKind::Remote => {
                child.remote = Some(RemoteResourceStorageConfig::parse(
                    &raw.remote.unwrap_or_default(),
                )?)
            }
            HybridChildBackendKind::Database => {
                child.database = Some(DatabaseStorageConfig::parse(
                    &raw.database.unwrap_or_default(),
                )?)
            }
        }
        Ok(child)
    }
}
