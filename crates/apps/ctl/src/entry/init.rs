use std::path::{Path, PathBuf};

use config_core::daemon::{OperatorConfig, OperatorConfigInitStatus};

use crate::args::InitMode;

mod document;

use document::DeliveryConfigDocument;

pub(super) struct OperatorConfigInitializer;

impl OperatorConfigInitializer {
    pub(super) fn write(
        path: &Path,
        mode: Option<InitMode>,
        force: bool,
        filter: bool,
        patch_paths: &[PathBuf],
    ) -> Result<OperatorConfigInitStatus, String> {
        let existed = path.exists();
        if existed && !force {
            if filter || mode.is_some() || !patch_paths.is_empty() {
                return Err(format!(
                    "config {} already exists; pass --force to apply --filter, --mode or --patch",
                    path.display()
                ));
            }
            OperatorConfig::load(path)
                .map_err(|error| format!("validate config {}: {error}", path.display()))?;
            return Ok(OperatorConfigInitStatus::ExistingValid);
        }
        let mut document = DeliveryConfigDocument::defaults(filter)?;
        if let Some(mode) = mode {
            document.apply_mode(mode)?;
        }
        document.patch_files(patch_paths).map_err(|error| format!(
            "initialize configuration {} with --patch {patch_paths:?} failed: {error}; check each --patch path and read permissions, then correct the patch files",
            path.display()
        ))?;
        document.write(path, force).map_err(|error| format!(
            "write configuration {} failed: {error}; check the configuration values and patch contents, destination path and write permissions",
            path.display()
        ))?;
        Ok(if existed {
            OperatorConfigInitStatus::Overwritten
        } else {
            OperatorConfigInitStatus::Created
        })
    }
}
