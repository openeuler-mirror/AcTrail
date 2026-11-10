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
        document.patch_files(patch_paths)?;
        document.write(path, force)?;
        Ok(if existed {
            OperatorConfigInitStatus::Overwritten
        } else {
            OperatorConfigInitStatus::Created
        })
    }
}
