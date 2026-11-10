use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::daemon::operator::{CheckpointBackendConfig, CheckpointRuntimeConfig};

use super::require_positive_u64;

/// `[checkpoint]`. `catalog_path` is the SQLite file of the checkpoint catalog; `backends` lists
/// the integrations a Trace is composed of, in call order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct CheckpointDocument {
    enabled: bool,
    catalog_path: String,
    backends: Vec<CheckpointBackendDocument>,
    runtime: CheckpointRuntimeConfig,
}

/// `[[checkpoint.backends]]`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckpointBackendDocument {
    name: String,
    library: String,
    config: String,
}

impl Default for CheckpointDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            catalog_path: "/var/lib/actrail/checkpoint.db".to_string(),
            backends: Vec::new(),
            runtime: CheckpointRuntimeConfig::default(),
        }
    }
}

impl CheckpointDocument {
    pub(super) fn from_config(
        catalog_path: Option<&Path>,
        backends: &[CheckpointBackendConfig],
        runtime: CheckpointRuntimeConfig,
    ) -> Self {
        match catalog_path {
            Some(path) => Self {
                enabled: true,
                runtime,
                catalog_path: path.display().to_string(),
                backends: backends
                    .iter()
                    .map(|backend| CheckpointBackendDocument {
                        name: backend.name.clone(),
                        library: backend.library.display().to_string(),
                        config: backend.config.display().to_string(),
                    })
                    .collect(),
            },
            None => Self {
                runtime,
                ..Self::default()
            },
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn runtime(&self) -> Result<CheckpointRuntimeConfig, String> {
        for (key, value) in [
            ("busy_timeout_ms", self.runtime.busy_timeout_ms),
            ("stop_timeout_ms", self.runtime.stop_timeout_ms),
            ("stop_poll_interval_ms", self.runtime.stop_poll_interval_ms),
            (
                "shutdown_drain_timeout_ms",
                self.runtime.shutdown_drain_timeout_ms,
            ),
            ("idle_poll_interval_ms", self.runtime.idle_poll_interval_ms),
        ] {
            require_positive_u64(&format!("checkpoint.runtime.{key}"), value)?;
        }
        Ok(self.runtime)
    }

    pub(super) fn to_config(&self) -> Result<Option<PathBuf>, String> {
        if !self.enabled {
            return Ok(None);
        }
        let path = PathBuf::from(&self.catalog_path);
        if !path.is_absolute() {
            return Err("checkpoint.catalog_path must be an absolute path".to_string());
        }
        Ok(Some(path))
    }

    pub(super) fn backends(&self) -> Result<Vec<CheckpointBackendConfig>, String> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        self.backends
            .iter()
            .map(|backend| {
                let library = PathBuf::from(&backend.library);
                let config = PathBuf::from(&backend.config);
                if !library.is_absolute() || !config.is_absolute() {
                    return Err(format!(
                        "checkpoint.backends {}: library and config must be absolute paths",
                        backend.name
                    ));
                }
                Ok(CheckpointBackendConfig {
                    name: backend.name.clone(),
                    library,
                    config,
                })
            })
            .collect()
    }
}
