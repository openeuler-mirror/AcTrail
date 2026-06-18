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
            CleanEntry::file_like("storage_sqlite_path", config.storage.path().to_path_buf()),
            CleanEntry::file_like("log_path", config.log_path.clone()),
            CleanEntry::directory(
                "export_directory",
                config.export_config.output_directory.clone(),
            ),
        ];
        for output_file in config.export_runtime.enabled_output_files() {
            entries.push(CleanEntry::file_like(output_file.label, output_file.path));
        }
        if config.payload_config.tls.capture_backend.is_sync() {
            entries.push(CleanEntry::file_like(
                "payload_tls_sync_event_socket_path",
                config.payload_config.tls.sync_event_socket_path.clone(),
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
    let mut removed_entries = 0_u64;
    let mut skipped_entries = 0_u64;
    let mut freed_bytes = 0_u64;
    for entry in artifacts.entries {
        match clean_entry(entry)? {
            CleanResult::Removed { bytes } => {
                removed_entries += 1;
                freed_bytes = freed_bytes.saturating_add(bytes);
            }
            CleanResult::SkippedMissing => {
                skipped_entries += 1;
            }
        }
    }
    println!(
        "clean summary: removed {} artifact(s), skipped {} missing, freed {}",
        removed_entries,
        skipped_entries,
        format_bytes(freed_bytes)
    );
    Ok(i32::default())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanResult {
    Removed { bytes: u64 },
    SkippedMissing,
}

fn clean_entry(entry: CleanEntry) -> Result<CleanResult, String> {
    validate_clean_path(&entry)?;
    let metadata = match fs::symlink_metadata(&entry.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("skipped missing {} {}", entry.label, entry.path.display());
            return Ok(CleanResult::SkippedMissing);
        }
        Err(error) => {
            return Err(format!(
                "inspect {} {}: {error}",
                entry.label,
                entry.path.display()
            ));
        }
    };
    let bytes = clean_entry_size(&entry, &metadata)?;
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
    println!(
        "removed {} {} (freed {})",
        entry.label,
        entry.path.display(),
        format_bytes(bytes)
    );
    Ok(CleanResult::Removed { bytes })
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

fn clean_entry_size(entry: &CleanEntry, metadata: &fs::Metadata) -> Result<u64, String> {
    match entry.kind {
        CleanKind::FileLike => Ok(metadata.len()),
        CleanKind::Directory => directory_size(&entry.path),
    }
}

fn directory_size(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("measure {}: {error}", path.display()))?;
    let mut size = metadata.len();
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(size);
    }
    for child in fs::read_dir(path)
        .map_err(|error| format!("measure directory {}: {error}", path.display()))?
    {
        let child =
            child.map_err(|error| format!("measure directory {}: {error}", path.display()))?;
        size = size.saturating_add(directory_size(&child.path())?);
    }
    Ok(size)
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB ({} bytes)", bytes as f64 / GIB as f64, bytes)
    } else if bytes >= MIB {
        format!("{:.2} MiB ({} bytes)", bytes as f64 / MIB as f64, bytes)
    } else if bytes >= KIB {
        format!("{:.2} KiB ({} bytes)", bytes as f64 / KIB as f64, bytes)
    } else {
        format!("{bytes} bytes")
    }
}
