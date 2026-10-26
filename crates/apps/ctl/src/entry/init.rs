use std::path::{Path, PathBuf};

use config_core::daemon::{OperatorConfig, OperatorConfigInitStatus};
use model_core::capability::Capability;

use crate::args::InitMode;

pub(super) struct OperatorConfigInitializer;

impl OperatorConfigInitializer {
    pub(super) fn write(
        path: &Path,
        mode: Option<InitMode>,
        force: bool,
        patch_paths: &[PathBuf],
    ) -> Result<OperatorConfigInitStatus, String> {
        let existed = path.exists();
        if existed && !force {
            if mode.is_some() || !patch_paths.is_empty() {
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
        if !patch_paths.is_empty() {
            config = config.patch_files(patch_paths)?;
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
        match mode {
            InitMode::Complete => config = config.patch(include_str!("complete.toml"))?,
            InitMode::Profile => {
                config.capture_profile.capabilities.retain(|request| {
                    !matches!(
                        request.capability,
                        Capability::EnforcementFilePermissionFanotify
                            | Capability::EnforcementFileMkdirSeccomp
                            | Capability::EnforcementFileRmdirSeccomp
                            | Capability::EnforcementCommandExecutionSeccomp
                            | Capability::EnforcementNetworkConnectSeccomp
                            | Capability::EnforcementFilePermissionLsm
                            | Capability::EnforcementCommandExecutionLsm
                            | Capability::EnforcementNetworkConnectLsm
                    )
                });
                config = config.patch(include_str!("profile.toml"))?;
            }
        }
        Ok(config)
    }
}
