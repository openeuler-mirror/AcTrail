use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileObservationDocument {
    pub enabled: bool,
    pub metadata_retention: String,
    pub tty: FileTtyDocument,
    pub bulk_read: FileBulkReadDocument,
    pub enumerate: FileEnumerateDocument,
}

impl Default for FileObservationDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            metadata_retention: "compact".to_string(),
            tty: FileTtyDocument::default(),
            bulk_read: FileBulkReadDocument::default(),
            enumerate: FileEnumerateDocument::default(),
        }
    }
}

impl FileObservationDocument {
    pub(super) fn from_config(config: &FileObservationConfig) -> Self {
        Self {
            enabled: config.enabled,
            metadata_retention: file_metadata_retention_as_str(config.metadata_retention)
                .to_string(),
            tty: FileTtyDocument {
                enabled: config.tty.enabled,
                paths: config.tty.paths.clone(),
                operations: config.tty.operations.clone(),
                raw_event_retention: file_raw_event_retention_as_str(
                    config.tty.raw_event_retention,
                )
                .to_string(),
                summary_flush_interval_ms: config.tty.summary_flush_interval_ms,
            },
            bulk_read: FileBulkReadDocument {
                enabled: config.bulk_read.enabled,
                mode: config.bulk_read.mode.as_str().to_string(),
                raw_event_retention: file_raw_event_retention_as_str(
                    config.bulk_read.raw_event_retention,
                )
                .to_string(),
                min_unique_paths: config.bulk_read.min_unique_paths,
                max_paths_per_set: config.bulk_read.max_paths_per_set,
                path_set_chunk_max_paths: config.bulk_read.path_set_chunk_max_paths,
                pending_event_max: config.bulk_read.pending_event_max,
            },
            enumerate: FileEnumerateDocument {
                enabled: config.enumerate.enabled,
                raw_event_retention: file_raw_event_retention_as_str(
                    config.enumerate.raw_event_retention,
                )
                .to_string(),
                min_unique_paths: config.enumerate.min_unique_paths,
                max_paths_per_set: config.enumerate.max_paths_per_set,
                path_set_chunk_max_paths: config.enumerate.path_set_chunk_max_paths,
            },
        }
    }

    pub(super) fn to_config(&self) -> Result<FileObservationConfig, String> {
        let config = FileObservationConfig {
            enabled: self.enabled,
            metadata_retention: parse_value(
                "file_observation.metadata_retention",
                &self.metadata_retention,
            )?,
            tty: self.tty.to_config()?,
            bulk_read: self.bulk_read.to_config()?,
            enumerate: self.enumerate.to_config()?,
        };
        if config.bulk_read.max_paths_per_set < config.bulk_read.min_unique_paths {
            return Err(
                "file_observation.bulk_read.max_paths_per_set must be >= file_observation.bulk_read.min_unique_paths"
                    .to_string(),
            );
        }
        if config.enumerate.max_paths_per_set < config.enumerate.min_unique_paths {
            return Err(
                "file_observation.enumerate.max_paths_per_set must be >= file_observation.enumerate.min_unique_paths"
                    .to_string(),
            );
        }
        Ok(config)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileTtyDocument {
    pub enabled: bool,
    pub paths: Vec<String>,
    pub operations: Vec<String>,
    pub raw_event_retention: String,
    pub summary_flush_interval_ms: u32,
}

impl Default for FileTtyDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            paths: ["/dev/tty", "/dev/pts/*"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            operations: [
                "open", "close", "read", "readv", "write", "writev", "truncate",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            raw_event_retention: "summary".to_string(),
            summary_flush_interval_ms: 5000,
        }
    }
}

impl FileTtyDocument {
    pub(super) fn to_config(&self) -> Result<FileTtyObservationConfig, String> {
        if self.paths.iter().any(|path| path.is_empty()) {
            return Err("file_observation.tty.paths must not contain empty entries".to_string());
        }
        if self.operations.iter().any(|operation| operation.is_empty()) {
            return Err(
                "file_observation.tty.operations must not contain empty entries".to_string(),
            );
        }
        Ok(FileTtyObservationConfig {
            enabled: self.enabled,
            paths: self.paths.clone(),
            operations: self.operations.clone(),
            raw_event_retention: parse_value(
                "file_observation.tty.raw_event_retention",
                &self.raw_event_retention,
            )?,
            summary_flush_interval_ms: require_positive_u32(
                "file_observation.tty.summary_flush_interval_ms",
                self.summary_flush_interval_ms,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileBulkReadDocument {
    pub enabled: bool,
    pub mode: String,
    pub raw_event_retention: String,
    pub min_unique_paths: u32,
    pub max_paths_per_set: u32,
    pub path_set_chunk_max_paths: u32,
    pub pending_event_max: u32,
}

impl Default for FileBulkReadDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "path_set".to_string(),
            raw_event_retention: "errors_only".to_string(),
            min_unique_paths: 16,
            max_paths_per_set: 4096,
            path_set_chunk_max_paths: 256,
            pending_event_max: 256,
        }
    }
}

impl FileBulkReadDocument {
    pub(super) fn to_config(&self) -> Result<FileBulkReadObservationConfig, String> {
        Ok(FileBulkReadObservationConfig {
            enabled: self.enabled,
            mode: parse_value("file_observation.bulk_read.mode", &self.mode)?,
            raw_event_retention: parse_value(
                "file_observation.bulk_read.raw_event_retention",
                &self.raw_event_retention,
            )?,
            min_unique_paths: require_positive_u32(
                "file_observation.bulk_read.min_unique_paths",
                self.min_unique_paths,
            )?,
            max_paths_per_set: require_positive_u32(
                "file_observation.bulk_read.max_paths_per_set",
                self.max_paths_per_set,
            )?,
            path_set_chunk_max_paths: require_positive_u32(
                "file_observation.bulk_read.path_set_chunk_max_paths",
                self.path_set_chunk_max_paths,
            )?,
            pending_event_max: require_positive_u32(
                "file_observation.bulk_read.pending_event_max",
                self.pending_event_max,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileEnumerateDocument {
    pub enabled: bool,
    pub raw_event_retention: String,
    pub min_unique_paths: u32,
    pub max_paths_per_set: u32,
    pub path_set_chunk_max_paths: u32,
}

impl Default for FileEnumerateDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            raw_event_retention: "errors_only".to_string(),
            min_unique_paths: 2,
            max_paths_per_set: 4096,
            path_set_chunk_max_paths: 256,
        }
    }
}

impl FileEnumerateDocument {
    pub(super) fn to_config(&self) -> Result<FsEnumerateObservationConfig, String> {
        Ok(FsEnumerateObservationConfig {
            enabled: self.enabled,
            raw_event_retention: parse_value(
                "file_observation.enumerate.raw_event_retention",
                &self.raw_event_retention,
            )?,
            min_unique_paths: require_positive_u32(
                "file_observation.enumerate.min_unique_paths",
                self.min_unique_paths,
            )?,
            max_paths_per_set: require_positive_u32(
                "file_observation.enumerate.max_paths_per_set",
                self.max_paths_per_set,
            )?,
            path_set_chunk_max_paths: require_positive_u32(
                "file_observation.enumerate.path_set_chunk_max_paths",
                self.path_set_chunk_max_paths,
            )?,
        })
    }
}
