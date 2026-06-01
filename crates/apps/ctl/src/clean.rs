//! Local cleanup for operator-configured runtime artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use config_core::daemon::OperatorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanArtifacts {
    entries: Vec<CleanEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CleanEntry {
    label: &'static str,
    path: PathBuf,
    kind: CleanKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanKind {
    FileLike,
    Directory,
}

impl CleanArtifacts {
    pub(crate) fn from_config(config: &OperatorConfig) -> Self {
        let mut entries = vec![
            CleanEntry::file_like("socket_path", config.socket_path.clone()),
            CleanEntry::file_like("pid_file", config.pid_file.clone()),
            CleanEntry::file_like("storage_path", config.storage_path.clone()),
            CleanEntry::file_like("log_path", config.log_path.clone()),
            CleanEntry::directory(
                "export_directory",
                config.export_config.output_directory.clone(),
            ),
        ];
        if config.live_otel_export.enabled {
            entries.push(CleanEntry::file_like(
                "otel_live_export_path",
                config.live_otel_export.path.clone(),
            ));
        }
        Self { entries }
    }
}

impl CleanEntry {
    fn file_like(label: &'static str, path: PathBuf) -> Self {
        Self {
            label,
            path,
            kind: CleanKind::FileLike,
        }
    }

    fn directory(label: &'static str, path: PathBuf) -> Self {
        Self {
            label,
            path,
            kind: CleanKind::Directory,
        }
    }
}

pub(crate) fn run_clean(artifacts: CleanArtifacts) -> Result<i32, String> {
    for entry in artifacts.entries {
        clean_entry(entry)?;
    }
    Ok(i32::default())
}

fn clean_entry(entry: CleanEntry) -> Result<(), String> {
    validate_clean_path(&entry)?;
    let metadata = match fs::symlink_metadata(&entry.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("skipped missing {} {}", entry.label, entry.path.display());
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "inspect {} {}: {error}",
                entry.label,
                entry.path.display()
            ));
        }
    };
    match entry.kind {
        CleanKind::FileLike => {
            if metadata.is_dir() {
                return Err(format!(
                    "refusing to remove directory for {} {}",
                    entry.label,
                    entry.path.display()
                ));
            }
            fs::remove_file(&entry.path).map_err(|error| {
                format!("remove {} {}: {error}", entry.label, entry.path.display())
            })?;
        }
        CleanKind::Directory => {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "refusing to remove symlinked directory for {} {}",
                    entry.label,
                    entry.path.display()
                ));
            }
            if !metadata.is_dir() {
                return Err(format!(
                    "refusing to remove non-directory for {} {}",
                    entry.label,
                    entry.path.display()
                ));
            }
            fs::remove_dir_all(&entry.path).map_err(|error| {
                format!("remove {} {}: {error}", entry.label, entry.path.display())
            })?;
        }
    }
    println!("removed {} {}", entry.label, entry.path.display());
    Ok(())
}

fn validate_clean_path(entry: &CleanEntry) -> Result<(), String> {
    if !entry.path.is_absolute() {
        return Err(format!(
            "refusing to clean non-absolute {} {}",
            entry.label,
            entry.path.display()
        ));
    }
    if entry.path == Path::new("/") {
        return Err(format!("refusing to clean root path for {}", entry.label));
    }
    Ok(())
}
