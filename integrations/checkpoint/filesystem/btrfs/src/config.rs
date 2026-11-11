use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use checkpoint::BackendError;
use serde::Deserialize;

use crate::{backend_error, btrfs, io_error};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BtrfsConfig {
    pub(crate) btrfs_path: PathBuf,
    pub(crate) findmnt_path: PathBuf,
    #[serde(default = "default_copy_path")]
    pub(crate) copy_path: PathBuf,
    #[serde(default = "default_copy_batch_max_bytes")]
    pub(crate) copy_batch_max_bytes: usize,
    /// Directory on the workspace filesystem that holds the snapshots.
    pub(crate) snapshot_root: PathBuf,
    /// Scope roots must lie under one of these directories.
    pub(crate) allowed_workspace_roots: Vec<PathBuf>,
}

impl BtrfsConfig {
    pub(crate) fn parse(json: &str) -> Result<Self, BackendError> {
        let mut config: Self = serde_json::from_str(json)
            .map_err(|error| backend_error("invalid_config", error.to_string()))?;
        if config.copy_batch_max_bytes == 0 {
            return Err(invalid("copy_batch_max_bytes must be positive"));
        }
        config.btrfs_path = executable(&config.btrfs_path, "btrfs_path")?;
        config.findmnt_path = executable(&config.findmnt_path, "findmnt_path")?;
        config.copy_path = executable(&config.copy_path, "copy_path")?;
        config.snapshot_root = canonical_dir(&config.snapshot_root, "snapshot_root")?;
        btrfs::require_btrfs(&config.snapshot_root)?;
        if config.allowed_workspace_roots.is_empty() {
            return Err(invalid("allowed_workspace_roots must not be empty"));
        }
        config.allowed_workspace_roots = config
            .allowed_workspace_roots
            .iter()
            .map(|root| canonical_dir(root, "allowed_workspace_roots"))
            .collect::<Result<_, _>>()?;
        for root in &config.allowed_workspace_roots {
            if root.starts_with(&config.snapshot_root) || config.snapshot_root.starts_with(root) {
                return Err(invalid(
                    "snapshot_root must not overlap an allowed workspace root",
                ));
            }
        }
        Ok(config)
    }
}

fn default_copy_path() -> PathBuf {
    PathBuf::from("/usr/bin/cp")
}

fn default_copy_batch_max_bytes() -> usize {
    64 * 1024
}

fn executable(path: &Path, label: &str) -> Result<PathBuf, BackendError> {
    if !path.is_absolute() {
        return Err(invalid(format!("{label} must be absolute")));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| io_error(path.display(), error))?;
    let metadata = fs::metadata(&canonical).map_err(|error| io_error(path.display(), error))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(invalid(format!("{label} must be an executable file")));
    }
    // Validate the target, but preserve the invoked name for multi-call executables.
    Ok(path.to_path_buf())
}

fn canonical_dir(path: &Path, label: &str) -> Result<PathBuf, BackendError> {
    if !path.is_absolute() {
        return Err(invalid(format!("{label} must be absolute")));
    }
    path.canonicalize()
        .map_err(|error| io_error(path.display(), error))
}

fn invalid(message: impl Into<String>) -> BackendError {
    backend_error("invalid_config", message)
}
