use super::HybridChildConfig;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HybridResourceStorageConfig {
    pub queue_capacity_batches: u32,
    pub write_timeout_ms: u64,
    pub backends: Vec<HybridChildConfig>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    queue_capacity_batches: Option<u32>,
    write_timeout_ms: Option<u64>,
    backends: Vec<toml::Table>,
}
impl HybridResourceStorageConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource hybrid: {e}"))?;
        let backends: Vec<_> = raw
            .backends
            .iter()
            .map(HybridChildConfig::parse)
            .collect::<Result<_, _>>()?;
        let queue_capacity_batches = raw.queue_capacity_batches.unwrap_or(64);
        let write_timeout_ms = raw.write_timeout_ms.unwrap_or(1000);
        if backends.is_empty() || queue_capacity_batches == 0 || write_timeout_ms == 0 {
            return Err(
                "resource hybrid requires children and positive queue/timeout values".into(),
            );
        }
        let mut names = std::collections::HashSet::new();
        if backends.iter().any(|b| !names.insert(&b.name)) {
            return Err("resource hybrid child names must be unique".into());
        }
        Ok(Self {
            queue_capacity_batches,
            write_timeout_ms,
            backends,
        })
    }
    pub fn local_files(&self) -> Vec<std::path::PathBuf> {
        self.backends
            .iter()
            .filter_map(|child| child.database.as_ref())
            .flat_map(|database| database.local_files())
            .collect()
    }
    pub fn resolve_paths(&mut self, base: &std::path::Path) {
        for child in &mut self.backends {
            if let Some(db) = &mut child.database {
                db.resolve_paths(base);
            }
        }
    }
}
