//! Local cleanup for operator-configured runtime artifacts.

use std::fs;
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use config_core::daemon::OperatorConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanArtifacts {
    entries: Vec<CleanEntry>,
    daemon_pid_file: PathBuf,
    daemon_socket_path: PathBuf,
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
        Self {
            entries,
            daemon_pid_file: config.pid_file.clone(),
            daemon_socket_path: config.socket_path.clone(),
        }
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
    ensure_daemon_not_running(&artifacts)?;
    let mut stats = CleanStats::default();
    for node in build_clean_tree(artifacts.entries) {
        clean_node(&node, 0, &mut stats)?;
    }
    println!(
        "clean summary: removed {} artifact(s), skipped {} missing, freed {}",
        stats.removed_entries,
        stats.skipped_entries,
        format_bytes(stats.freed_bytes)
    );
    Ok(i32::default())
}

fn ensure_daemon_not_running(artifacts: &CleanArtifacts) -> Result<(), String> {
    if let Some(pid) = read_pid_file(&artifacts.daemon_pid_file)?
        && process_exists(pid)?
    {
        return Err(format!(
            "refusing to clean while actraild appears to be running pid={pid}; run `actraild stop` first"
        ));
    }
    if socket_has_active_listener(&artifacts.daemon_socket_path)? {
        return Err(format!(
            "refusing to clean while actraild socket {} has an active listener; run `actraild stop` first",
            artifacts.daemon_socket_path.display()
        ));
    }
    Ok(())
}

fn read_pid_file(path: &Path) -> Result<Option<u32>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read pid file {}: {error}", path.display())),
    };
    let pid = raw
        .trim()
        .parse::<u32>()
        .map_err(|error| format!("invalid pid file {}: {error}", path.display()))?;
    if pid == u32::default() {
        return Err(format!(
            "invalid pid file {}: pid must not be zero",
            path.display()
        ));
    }
    Ok(Some(pid))
}

fn process_exists(pid: u32) -> Result<bool, String> {
    let raw_pid =
        libc::pid_t::try_from(pid).map_err(|error| format!("invalid pid {pid}: {error}"))?;
    let result = unsafe { libc::kill(raw_pid, libc::c_int::default()) };
    if result == libc::c_int::default() {
        return Ok(true);
    }
    match io::Error::last_os_error().raw_os_error() {
        Some(errno) if errno == libc::ESRCH => Ok(false),
        Some(errno) if errno == libc::EPERM => Ok(true),
        Some(errno) => Err(format!("check pid={pid} failed with errno {errno}")),
        None => Err(format!("check pid={pid} failed")),
    }
}

fn socket_has_active_listener(path: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("inspect socket {}: {error}", path.display())),
    };
    if !metadata.file_type().is_socket() {
        return Ok(false);
    }
    match UnixStream::connect(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(false),
        Err(error) => Err(format!(
            "refusing to clean because socket {} could not be verified stale: {error}",
            path.display()
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CleanNode {
    entry: CleanEntry,
    children: Vec<CleanNode>,
}

fn build_clean_tree(entries: Vec<CleanEntry>) -> Vec<CleanNode> {
    let parents = parent_indices(&entries);
    build_clean_children(None, &entries, &parents)
}

fn parent_indices(entries: &[CleanEntry]) -> Vec<Option<usize>> {
    entries
        .iter()
        .enumerate()
        .map(|(entry_index, entry)| {
            entries
                .iter()
                .enumerate()
                .filter(|(candidate_index, candidate)| {
                    *candidate_index != entry_index
                        && candidate.kind == CleanKind::Directory
                        && path_is_child_of(&entry.path, &candidate.path)
                })
                .max_by_key(|(_, candidate)| candidate.path.components().count())
                .map(|(candidate_index, _)| candidate_index)
        })
        .collect()
}

fn path_is_child_of(path: &Path, parent: &Path) -> bool {
    path != parent && path.starts_with(parent)
}

fn build_clean_children(
    parent: Option<usize>,
    entries: &[CleanEntry],
    parents: &[Option<usize>],
) -> Vec<CleanNode> {
    entries
        .iter()
        .enumerate()
        .filter(|(entry_index, _)| parents[*entry_index] == parent)
        .map(|(entry_index, entry)| CleanNode {
            entry: entry.clone(),
            children: build_clean_children(Some(entry_index), entries, parents),
        })
        .collect()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CleanStats {
    removed_entries: u64,
    skipped_entries: u64,
    freed_bytes: u64,
}

impl CleanStats {
    fn record(&mut self, result: CleanResult) {
        match result {
            CleanResult::Removed { bytes } => {
                self.removed_entries += 1;
                self.freed_bytes = self.freed_bytes.saturating_add(bytes);
            }
            CleanResult::SkippedMissing => {
                self.skipped_entries += 1;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CleanResult {
    Removed { bytes: u64 },
    SkippedMissing,
}

fn clean_node(node: &CleanNode, depth: usize, stats: &mut CleanStats) -> Result<(), String> {
    if node.children.is_empty() {
        let result = clean_entry(&node.entry, depth)?;
        stats.record(result);
        return Ok(());
    }
    println!(
        "{}- Clear {} {}",
        indent(depth),
        node.entry.label,
        node.entry.path.display()
    );
    let Some(metadata) = inspect_entry(&node.entry)? else {
        let result = CleanResult::SkippedMissing;
        print_clean_result(&node.entry, depth + 1, result);
        stats.record(result);
        return Ok(());
    };
    validate_present_entry(&node.entry, &metadata)?;
    for child in &node.children {
        clean_node(child, depth + 1, stats)?;
    }
    let result = remove_present_entry(&node.entry, depth + 1)?;
    stats.record(result);
    Ok(())
}

fn clean_entry(entry: &CleanEntry, depth: usize) -> Result<CleanResult, String> {
    let Some(metadata) = inspect_entry(entry)? else {
        let result = CleanResult::SkippedMissing;
        print_clean_result(entry, depth, result);
        return Ok(result);
    };
    validate_present_entry(entry, &metadata)?;
    remove_present_entry(entry, depth)
}

fn inspect_entry(entry: &CleanEntry) -> Result<Option<fs::Metadata>, String> {
    validate_clean_path(entry)?;
    let metadata = match fs::symlink_metadata(&entry.path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "inspect {} {}: {error}",
                entry.label,
                entry.path.display()
            ));
        }
    };
    Ok(Some(metadata))
}

fn validate_present_entry(entry: &CleanEntry, metadata: &fs::Metadata) -> Result<(), String> {
    match entry.kind {
        CleanKind::FileLike => {
            if metadata.is_dir() {
                return Err(format!(
                    "refusing to remove directory for {} {}",
                    entry.label,
                    entry.path.display()
                ));
            }
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
        }
    }
    Ok(())
}

fn remove_present_entry(entry: &CleanEntry, depth: usize) -> Result<CleanResult, String> {
    let metadata = fs::symlink_metadata(&entry.path)
        .map_err(|error| format!("inspect {} {}: {error}", entry.label, entry.path.display()))?;
    validate_present_entry(entry, &metadata)?;
    let bytes = clean_entry_size(&entry, &metadata)?;
    match entry.kind {
        CleanKind::FileLike => {
            fs::remove_file(&entry.path).map_err(|error| {
                format!("remove {} {}: {error}", entry.label, entry.path.display())
            })?;
        }
        CleanKind::Directory => {
            fs::remove_dir_all(&entry.path).map_err(|error| {
                format!("remove {} {}: {error}", entry.label, entry.path.display())
            })?;
        }
    }
    let result = CleanResult::Removed { bytes };
    print_clean_result(entry, depth, result);
    Ok(result)
}

fn print_clean_result(entry: &CleanEntry, depth: usize, result: CleanResult) {
    match result {
        CleanResult::Removed { bytes } => println!(
            "{}- Clear {} {} Done. Size: {}",
            indent(depth),
            entry.label,
            entry.path.display(),
            format_bytes(bytes)
        ),
        CleanResult::SkippedMissing => println!(
            "{}- Clear {} {} Skip. Size: 0 bytes (missing)",
            indent(depth),
            entry.label,
            entry.path.display()
        ),
    }
}

fn indent(depth: usize) -> String {
    "  ".repeat(depth)
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
