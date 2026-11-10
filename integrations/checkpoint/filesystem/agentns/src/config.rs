use std::path::PathBuf;

use checkpoint::BackendError;
use serde::Deserialize;

use crate::backend_error;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentNsConfig {
    /// Each connection gets a private directory here that names its RNS area.
    pub(crate) area_root: PathBuf,
    /// `librns.so` of the RNS installation the running kernel module belongs to.
    pub(crate) library_path: PathBuf,
    /// Never captured, in addition to the scope's excludes and the areas.
    #[serde(default)]
    pub(crate) extra_excluded_paths: Vec<PathBuf>,
}

impl AgentNsConfig {
    pub(crate) fn parse(json: &str) -> Result<Self, BackendError> {
        let config: Self = serde_json::from_str(json)
            .map_err(|error| backend_error("invalid_config", error.to_string()))?;
        let paths = [
            ("area_root", &config.area_root),
            ("library_path", &config.library_path),
        ];
        let extra = config
            .extra_excluded_paths
            .iter()
            .map(|path| ("extra_excluded_paths", path));
        for (label, path) in paths.into_iter().chain(extra) {
            if !path.is_absolute() {
                return Err(backend_error(
                    "invalid_config",
                    format!("{label} must be absolute"),
                ));
            }
        }
        Ok(config)
    }
}
