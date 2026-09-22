use std::path::Path;

use config_core::daemon::{OperatorConfig, OperatorConfigInitStatus};
use model_core::capability::Capability;

use crate::args::InitMode;

pub(super) struct OperatorConfigInitializer;

impl OperatorConfigInitializer {
    pub(super) fn write(
        path: &Path,
        mode: Option<InitMode>,
        force: bool,
        patch_path: Option<&Path>,
    ) -> Result<OperatorConfigInitStatus, String> {
        let existed = path.exists();
        if existed && !force {
            if mode.is_some() || patch_path.is_some() {
                return Err(format!(
                    "config {} already exists; pass --force to apply --mode or --patch",
                    path.display()
                ));
            }
            OperatorConfig::load(path)
                .map_err(|error| format!("validate config {}: {error}", path.display()))?;
            return Ok(OperatorConfigInitStatus::ExistingValid);
        }
        let mut config = Self::configuration(mode.unwrap_or_default())?;
        if let Some(patch_path) = patch_path {
            config = config.patch_file(patch_path)?;
        }
        config.dump_to_path(path, force)?;
        Ok(if existed {
            OperatorConfigInitStatus::Overwritten
        } else {
            OperatorConfigInitStatus::Created
        })
    }

    fn configuration(mode: InitMode) -> Result<OperatorConfig, String> {
        let mut config = OperatorConfig::init()?;
        if mode == InitMode::Profile {
            config.capture_profile.capabilities.retain(|request| {
                !matches!(
                    request.capability,
                    Capability::EnforcementFilePermissionFanotify
                        | Capability::EnforcementCommandExecutionSeccomp
                        | Capability::EnforcementNetworkConnectSeccomp
                )
            });
            config = config.patch(include_str!("profile.toml"))?;
        }
        Ok(config)
    }
}
