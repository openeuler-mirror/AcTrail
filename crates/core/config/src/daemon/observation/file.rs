//! File observation retention controls.

use std::str::FromStr;

pub const DEFAULT_FILE_BULK_READ_MIN_UNIQUE_PATHS: u32 = 16;
pub const DEFAULT_FILE_BULK_READ_MAX_PATHS_PER_SET: u32 = 4096;
pub const DEFAULT_FILE_BULK_READ_PATH_SET_CHUNK_MAX_PATHS: u32 = 256;
pub const DEFAULT_FILE_BULK_READ_PENDING_EVENT_MAX: u32 = 256;
pub const DEFAULT_FILE_BULK_READ_FAST_PATH_PROCESS_MAX_ENTRIES: u32 = 4096;
pub const DEFAULT_FILE_BULK_READ_FAST_PATH_FD_MAX_ENTRIES: u32 = 8192;
pub const DEFAULT_FILE_BULK_READ_FAST_PATH_PENDING_OP_MAX_ENTRIES: u32 = 8192;
pub const DEFAULT_FS_ENUMERATE_MIN_UNIQUE_PATHS: u32 = 2;
pub const DEFAULT_FS_ENUMERATE_MAX_PATHS_PER_SET: u32 = 4096;
pub const DEFAULT_FS_ENUMERATE_PATH_SET_CHUNK_MAX_PATHS: u32 = 256;
pub const DEFAULT_FILE_TTY_SUMMARY_FLUSH_INTERVAL_MS: u32 = 5000;

const DEFAULT_TTY_PATHS: &[&str] = &["/dev/tty", "/dev/pts/*"];
const DEFAULT_TTY_OPERATIONS: &[&str] = &[
    "open", "close", "read", "readv", "write", "writev", "truncate",
];
const DEFAULT_FILE_BULK_READ_FAST_PATH_SCANNER_COMMANDS: &[&str] =
    &["grep", "egrep", "fgrep", "rg", "find"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileObservationConfig {
    pub enabled: bool,
    pub metadata_retention: FileMetadataRetention,
    pub tty: FileTtyObservationConfig,
    pub bulk_read: FileBulkReadObservationConfig,
    pub enumerate: FsEnumerateObservationConfig,
}

impl Default for FileObservationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            metadata_retention: FileMetadataRetention::Compact,
            tty: FileTtyObservationConfig::default(),
            bulk_read: FileBulkReadObservationConfig::default(),
            enumerate: FsEnumerateObservationConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileMetadataRetention {
    Full,
    Compact,
}

impl FromStr for FileMetadataRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "full" => Ok(Self::Full),
            "compact" => Ok(Self::Compact),
            _ => Err("expected full or compact".to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileRawEventRetention {
    Full,
    ErrorsOnly,
    Summary,
}

impl FileRawEventRetention {
    pub const fn retains_success(self) -> bool {
        matches!(self, Self::Full)
    }

    pub const fn retains_error(self) -> bool {
        matches!(self, Self::Full | Self::ErrorsOnly)
    }
}

impl FromStr for FileRawEventRetention {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "full" => Ok(Self::Full),
            "errors_only" => Ok(Self::ErrorsOnly),
            "summary" => Ok(Self::Summary),
            _ => Err("expected full, errors_only, or summary".to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileBulkReadMode {
    Counter,
    PathSet,
}

impl FileBulkReadMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::PathSet => "path_set",
        }
    }
}

impl FromStr for FileBulkReadMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "counter" => Ok(Self::Counter),
            "path_set" => Ok(Self::PathSet),
            _ => Err("expected counter or path_set".to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileTtyObservationConfig {
    pub enabled: bool,
    pub paths: Vec<String>,
    pub operations: Vec<String>,
    pub raw_event_retention: FileRawEventRetention,
    pub summary_flush_interval_ms: u32,
}

impl Default for FileTtyObservationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            paths: DEFAULT_TTY_PATHS
                .iter()
                .map(|value| value.to_string())
                .collect(),
            operations: DEFAULT_TTY_OPERATIONS
                .iter()
                .map(|value| value.to_string())
                .collect(),
            raw_event_retention: FileRawEventRetention::Summary,
            summary_flush_interval_ms: DEFAULT_FILE_TTY_SUMMARY_FLUSH_INTERVAL_MS,
        }
    }
}

impl FileTtyObservationConfig {
    pub fn matches(&self, path: &str, operation: &str) -> bool {
        self.matches_path(path) && self.matches_operation(operation)
    }

    pub fn matches_operation(&self, operation: &str) -> bool {
        self.enabled
            && self
                .operations
                .iter()
                .any(|candidate| candidate == operation)
    }

    pub fn matches_path(&self, path: &str) -> bool {
        self.enabled && self.paths.iter().any(|pattern| path_matches(pattern, path))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileBulkReadObservationConfig {
    pub enabled: bool,
    pub mode: FileBulkReadMode,
    pub raw_event_retention: FileRawEventRetention,
    pub min_unique_paths: u32,
    pub max_paths_per_set: u32,
    pub path_set_chunk_max_paths: u32,
    pub pending_event_max: u32,
    pub fast_path: FileBulkReadFastPathConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileBulkReadFastPathConfig {
    pub enabled: bool,
    pub scanner_commands: Vec<String>,
    pub process_max_entries: u32,
    pub fd_max_entries: u32,
    pub pending_op_max_entries: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FsEnumerateObservationConfig {
    pub enabled: bool,
    pub raw_event_retention: FileRawEventRetention,
    pub min_unique_paths: u32,
    pub max_paths_per_set: u32,
    pub path_set_chunk_max_paths: u32,
}

impl Default for FsEnumerateObservationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            raw_event_retention: FileRawEventRetention::ErrorsOnly,
            min_unique_paths: DEFAULT_FS_ENUMERATE_MIN_UNIQUE_PATHS,
            max_paths_per_set: DEFAULT_FS_ENUMERATE_MAX_PATHS_PER_SET,
            path_set_chunk_max_paths: DEFAULT_FS_ENUMERATE_PATH_SET_CHUNK_MAX_PATHS,
        }
    }
}

impl Default for FileBulkReadObservationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: FileBulkReadMode::PathSet,
            raw_event_retention: FileRawEventRetention::ErrorsOnly,
            min_unique_paths: DEFAULT_FILE_BULK_READ_MIN_UNIQUE_PATHS,
            max_paths_per_set: DEFAULT_FILE_BULK_READ_MAX_PATHS_PER_SET,
            path_set_chunk_max_paths: DEFAULT_FILE_BULK_READ_PATH_SET_CHUNK_MAX_PATHS,
            pending_event_max: DEFAULT_FILE_BULK_READ_PENDING_EVENT_MAX,
            fast_path: FileBulkReadFastPathConfig::default(),
        }
    }
}

impl Default for FileBulkReadFastPathConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scanner_commands: DEFAULT_FILE_BULK_READ_FAST_PATH_SCANNER_COMMANDS
                .iter()
                .map(|value| value.to_string())
                .collect(),
            process_max_entries: DEFAULT_FILE_BULK_READ_FAST_PATH_PROCESS_MAX_ENTRIES,
            fd_max_entries: DEFAULT_FILE_BULK_READ_FAST_PATH_FD_MAX_ENTRIES,
            pending_op_max_entries: DEFAULT_FILE_BULK_READ_FAST_PATH_PENDING_OP_MAX_ENTRIES,
        }
    }
}

fn path_matches(pattern: &str, path: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        return path.starts_with(prefix);
    }
    pattern == path
}
