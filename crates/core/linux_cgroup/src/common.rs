//! Types and filesystem helpers shared by the cgroup v1 and v2 adapters.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::FileExt;
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub enum CgroupError {
    UnsafePath(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Malformed {
        file: &'static str,
        detail: String,
    },
    CounterOverflow {
        file: &'static str,
        counter: &'static str,
    },
    Unsupported(String),
}

impl fmt::Display for CgroupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafePath(detail) => write!(f, "unsafe cgroup path: {detail}"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(f, "cgroup {operation} {}: {source}", path.display()),
            Self::Malformed { file, detail } => write!(f, "malformed {file}: {detail}"),
            Self::CounterOverflow { file, counter } => {
                write!(f, "counter overflow while aggregating {file}:{counter}")
            }
            Self::Unsupported(detail) => write!(f, "unsupported cgroup configuration: {detail}"),
        }
    }
}

impl std::error::Error for CgroupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceScopePaths {
    pub relative_path: PathBuf,
    pub aggregate: PathBuf,
    pub workload: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedScopeDirectory {
    pub trace_id: u64,
    pub nonce: String,
    pub paths: TraceScopePaths,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ManagedScopeScan {
    pub managed: Vec<ManagedScopeDirectory>,
    pub unknown: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryEvents {
    pub low: Option<u64>,
    pub high: Option<u64>,
    pub max: Option<u64>,
    pub oom: Option<u64>,
    pub oom_kill: Option<u64>,
    pub oom_group_kill: Option<u64>,
    pub unknown: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CpuStat {
    pub usage_usec: u64,
    pub user_usec: Option<u64>,
    pub system_usec: Option<u64>,
    pub nr_throttled: Option<u64>,
    pub throttled_usec: Option<u64>,
    pub unknown: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IoStat {
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub unknown: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CgroupCounters {
    pub memory_current_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub memory_anon_bytes: Option<u64>,
    pub memory_file_bytes: Option<u64>,
    pub memory_swap_current_bytes: Option<u64>,
    pub memory_events: Option<MemoryEvents>,
    pub memory_events_local: Option<MemoryEvents>,
    pub cpu: CpuStat,
    pub io: Option<IoStat>,
    pub pids_current: Option<u64>,
    pub pids_peak: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct ParsedValues {
    pub(crate) known: BTreeMap<String, u64>,
    pub(crate) unknown: BTreeMap<String, String>,
}

pub(crate) fn parse_known_values(
    raw: &str,
    file: &'static str,
    known_keys: &[&str],
) -> Result<ParsedValues, CgroupError> {
    let known_keys = known_keys.iter().copied().collect::<BTreeSet<_>>();
    let mut known = BTreeMap::new();
    let mut unknown = BTreeMap::new();
    for (line_index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 2 {
            return malformed(file, line_index, "expected key and value");
        }
        let key = fields[0];
        let value = fields[1];
        if known_keys.contains(key) {
            if known.contains_key(key) {
                return malformed(file, line_index, &format!("duplicate {key}"));
            }
            let parsed = value
                .parse::<u64>()
                .map_err(|error| CgroupError::Malformed {
                    file,
                    detail: format!("line {} invalid {key}: {error}", line_index + 1),
                })?;
            known.insert(key.to_string(), parsed);
        } else {
            unknown.insert(key.to_string(), value.to_string());
        }
    }
    Ok(ParsedValues { known, unknown })
}

pub(crate) fn add_counter(
    total: &mut Option<u64>,
    raw: &str,
    file: &'static str,
    counter: &'static str,
) -> Result<(), CgroupError> {
    let value = raw.parse::<u64>().map_err(|error| CgroupError::Malformed {
        file,
        detail: format!("invalid {counter}: {error}"),
    })?;
    *total = Some(
        total
            .unwrap_or_default()
            .checked_add(value)
            .ok_or(CgroupError::CounterOverflow { file, counter })?,
    );
    Ok(())
}

pub(crate) fn malformed<T>(
    file: &'static str,
    zero_based_line: usize,
    detail: &str,
) -> Result<T, CgroupError> {
    Err(CgroupError::Malformed {
        file,
        detail: format!("line {}: {detail}", zero_based_line + 1),
    })
}

pub(crate) fn validate_absolute_path(path: &Path) -> Result<(), CgroupError> {
    if !path.is_absolute() {
        return Err(CgroupError::UnsafePath(format!(
            "{} is not absolute",
            path.display()
        )));
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(CgroupError::UnsafePath(format!(
                "{} contains traversal components",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_component(value: &str) -> Result<(), CgroupError> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.contains('/')
        || value.contains('\\')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(CgroupError::UnsafePath(format!(
            "invalid component {value:?}"
        )));
    }
    Ok(())
}

pub(crate) fn parse_managed_scope_name(name: &str) -> Result<(u64, String), CgroupError> {
    let rest = name.strip_prefix("trace-").ok_or_else(|| {
        CgroupError::UnsafePath(format!("scope name {name:?} lacks trace prefix"))
    })?;
    let (trace_id, nonce) = rest
        .split_once('-')
        .ok_or_else(|| CgroupError::UnsafePath(format!("scope name {name:?} lacks nonce")))?;
    let trace_id = trace_id.parse::<u64>().map_err(|error| {
        CgroupError::UnsafePath(format!("scope name {name:?} has invalid trace id: {error}"))
    })?;
    validate_component(nonce)?;
    Ok((trace_id, nonce.to_string()))
}

pub(crate) fn subtree_is_empty_recursive(directory: &Path) -> Result<bool, CgroupError> {
    if directory
        .symlink_metadata()
        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
    {
        return Ok(true);
    }
    require_real_directory(directory)?;
    if !read_pid_list(&directory.join("cgroup.procs"))?.is_empty() {
        return Ok(false);
    }
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error("scan cgroup subtree", directory, source))?
    {
        let entry =
            entry.map_err(|source| io_error("read cgroup subtree entry", directory, source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("inspect cgroup subtree entry", &path, source))?;
        if file_type.is_symlink() {
            return Err(CgroupError::UnsafePath(format!(
                "managed subtree contains symlink {}",
                path.display()
            )));
        }
        if file_type.is_dir() && !subtree_is_empty_recursive(&path)? {
            return Ok(false);
        }
    }
    Ok(true)
}

// Only rmdir cgroup directories, never unlink controller files or follow links.
// The kernel refuses populated cgroups; a race/partial failure is retried later.
pub(crate) fn remove_cgroup_subtree(directory: &Path) -> Result<(), CgroupError> {
    if directory
        .symlink_metadata()
        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    require_real_directory(directory)?;
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error("scan cgroup cleanup", directory, source))?
    {
        let entry =
            entry.map_err(|source| io_error("read cgroup cleanup entry", directory, source))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|source| io_error("inspect cleanup entry", &path, source))?;
        if kind.is_symlink() {
            return Err(CgroupError::UnsafePath(format!(
                "cgroup cleanup contains symlink {}",
                path.display()
            )));
        }
        if kind.is_dir() {
            remove_cgroup_subtree(&path)?;
        }
    }
    match fs::remove_dir(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error("remove cgroup directory", directory, source)),
    }
}

/// Read-only membership enumeration; does not require managed hierarchy ownership.
pub fn read_cgroup_subtree_pids(directory: &Path) -> Result<Vec<u32>, CgroupError> {
    require_real_directory(directory)?;
    let mut pids = BTreeSet::new();
    collect_subtree_pids_recursive(directory, &mut pids)?;
    Ok(pids.into_iter().collect())
}

pub(crate) fn collect_subtree_pids_recursive(
    directory: &Path,
    pids: &mut BTreeSet<u32>,
) -> Result<(), CgroupError> {
    pids.extend(read_pid_list(&directory.join("cgroup.procs"))?);
    for entry in fs::read_dir(directory)
        .map_err(|source| io_error("scan cgroup subtree", directory, source))?
    {
        let entry =
            entry.map_err(|source| io_error("read cgroup subtree entry", directory, source))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| io_error("inspect cgroup subtree entry", &path, source))?;
        if file_type.is_symlink() {
            return Err(CgroupError::UnsafePath(format!(
                "managed subtree contains symlink {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect_subtree_pids_recursive(&path, pids)?;
        }
    }
    Ok(())
}

pub(crate) fn unescape_mountinfo_path(raw: &str) -> Result<String, CgroupError> {
    let bytes = raw.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            output.push(bytes[index]);
            index += 1;
            continue;
        }
        let end = index.checked_add(4).ok_or_else(|| CgroupError::Malformed {
            file: "mountinfo",
            detail: "escape length overflow".to_string(),
        })?;
        let escape = bytes
            .get(index + 1..end)
            .ok_or_else(|| CgroupError::Malformed {
                file: "mountinfo",
                detail: format!("truncated escape in {raw:?}"),
            })?;
        let escaped = match escape {
            b"040" => b' ',
            b"011" => b'\t',
            b"012" => b'\n',
            b"134" => b'\\',
            _ => {
                return Err(CgroupError::Malformed {
                    file: "mountinfo",
                    detail: format!("unsupported escape in {raw:?}"),
                });
            }
        };
        output.push(escaped);
        index = end;
    }
    String::from_utf8(output).map_err(|error| CgroupError::Malformed {
        file: "mountinfo",
        detail: error.to_string(),
    })
}

pub(crate) fn read_pid_list(path: &Path) -> Result<Vec<u32>, CgroupError> {
    let raw = read_required(path, "cgroup.procs")?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.trim()
                .parse::<u32>()
                .map_err(|error| CgroupError::Malformed {
                    file: "cgroup.procs",
                    detail: error.to_string(),
                })
        })
        .collect()
}

pub(crate) fn require_real_directory(path: &Path) -> Result<(), CgroupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            CgroupError::UnsafePath(format!("{} is not a real directory", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(source) => Err(io_error("inspect required directory", path, source)),
    }
}

pub(crate) fn is_real_directory(path: &Path) -> Result<bool, CgroupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error("inspect directory", path, source)),
    }
}

pub(crate) fn create_checked_directory(path: &Path) -> Result<(), CgroupError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            CgroupError::UnsafePath(format!("{} is not a real directory", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_new_directory(path),
        Err(source) => Err(io_error("inspect", path, source)),
    }
}

pub(crate) fn create_new_directory(path: &Path) -> Result<(), CgroupError> {
    fs::create_dir(path).map_err(|source| io_error("create directory", path, source))
}

pub(crate) fn write_control_file(path: &Path, value: &[u8]) -> Result<(), CgroupError> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|source| io_error("open for write", path, source))?;
    file.write_all(value)
        .map_err(|source| io_error("write", path, source))
}

pub(crate) fn read_required(path: &Path, file: &'static str) -> Result<String, CgroupError> {
    fs::read_to_string(path).map_err(|source| CgroupError::Io {
        operation: file,
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn read_descriptor(
    descriptor: &File,
    path: &Path,
    file: &'static str,
) -> Result<String, CgroupError> {
    let mut bytes = Vec::new();
    let mut offset = 0_u64;
    loop {
        let mut chunk = [0_u8; 4096];
        let count = descriptor
            .read_at(&mut chunk, offset)
            .map_err(|source| CgroupError::Io {
                operation: file,
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        offset = offset
            .checked_add(count as u64)
            .ok_or_else(|| CgroupError::Io {
                operation: file,
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidData, "counter size overflow"),
            })?;
    }
    String::from_utf8(bytes).map_err(|source| CgroupError::Io {
        operation: file,
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

pub(crate) fn parse_single_u64(raw: &str, file: &'static str) -> Result<u64, CgroupError> {
    raw.trim()
        .parse::<u64>()
        .map_err(|error| CgroupError::Malformed {
            file,
            detail: error.to_string(),
        })
}

pub(crate) fn io_error(operation: &'static str, path: &Path, source: io::Error) -> CgroupError {
    CgroupError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}
