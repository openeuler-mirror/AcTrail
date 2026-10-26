use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileIoSummaryDocument {
    pub flush_interval_ms: u32,
    pub max_entries: u32,
    pub object_max_entries: u32,
}

impl Default for FileIoSummaryDocument {
    fn default() -> Self {
        Self::from_config(&FileIoSummaryConfig::default())
    }
}

impl FileIoSummaryDocument {
    fn from_config(config: &FileIoSummaryConfig) -> Self {
        Self {
            flush_interval_ms: config.flush_interval_ms,
            max_entries: config.max_entries,
            object_max_entries: config.object_max_entries,
        }
    }

    fn to_config(&self) -> Result<FileIoSummaryConfig, String> {
        Ok(FileIoSummaryConfig {
            flush_interval_ms: require_positive_u32(
                "file_observation.summary.flush_interval_ms",
                self.flush_interval_ms,
            )?,
            max_entries: require_positive_u32(
                "file_observation.summary.max_entries",
                self.max_entries,
            )?,
            object_max_entries: require_positive_u32(
                "file_observation.summary.object_max_entries",
                self.object_max_entries,
            )?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileCollectionDocument {
    pub writable_open: bool,
    pub path_mutations: bool,
    pub fd_mutations: bool,
    pub read: FileIoCollectionDocument,
    pub write: FileIoCollectionDocument,
}

impl Default for FileCollectionDocument {
    fn default() -> Self {
        Self::from_config(&FileCollectionConfig::default())
    }
}

impl FileCollectionDocument {
    fn from_config(config: &FileCollectionConfig) -> Self {
        Self {
            writable_open: config.writable_open,
            path_mutations: config.path_mutations,
            fd_mutations: config.fd_mutations,
            read: FileIoCollectionDocument::from_config(&config.read),
            write: FileIoCollectionDocument::from_config(&config.write),
        }
    }

    fn to_config(&self) -> FileCollectionConfig {
        FileCollectionConfig {
            writable_open: self.writable_open,
            path_mutations: self.path_mutations,
            fd_mutations: self.fd_mutations,
            read: self.read.to_config(),
            write: self.write.to_config(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileIoCollectionDocument {
    pub observed: bool,
    pub counts: bool,
    pub bytes: bool,
    pub errors: bool,
}

impl FileIoCollectionDocument {
    fn from_config(config: &FileIoCollectionConfig) -> Self {
        Self {
            observed: config.observed,
            counts: config.counts,
            bytes: config.bytes,
            errors: config.errors,
        }
    }

    fn to_config(&self) -> FileIoCollectionConfig {
        FileIoCollectionConfig {
            observed: self.observed,
            counts: self.counts,
            bytes: self.bytes,
            errors: self.errors,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileObservationDocument {
    pub enabled: bool,
    pub collection: FileCollectionDocument,
    pub summary: FileIoSummaryDocument,
    pub metadata_retention: String,
    pub tty: FileTtyDocument,
    pub bulk_read: FileBulkReadDocument,
    pub enumerate: FileEnumerateDocument,
}

impl Default for FileObservationDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            collection: FileCollectionDocument::default(),
            summary: FileIoSummaryDocument::default(),
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
            collection: FileCollectionDocument::from_config(&config.collection),
            summary: FileIoSummaryDocument::from_config(&config.summary),
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
            },
            bulk_read: FileBulkReadDocument {
                enabled: config.bulk_read.enabled,
                mode: config.bulk_read.mode.as_str().to_string(),
                raw_event_retention: file_raw_event_retention_as_str(
                    config.bulk_read.raw_event_retention,
                )
                .to_string(),
                max_paths_per_set: config.bulk_read.max_paths_per_set,
                path_set_chunk_max_paths: config.bulk_read.path_set_chunk_max_paths,
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
            collection: self.collection.to_config(),
            summary: self.summary.to_config()?,
            metadata_retention: parse_value(
                "file_observation.metadata_retention",
                &self.metadata_retention,
            )?,
            tty: self.tty.to_config()?,
            bulk_read: self.bulk_read.to_config()?,
            enumerate: self.enumerate.to_config()?,
        };
        if config.enumerate.max_paths_per_set < config.enumerate.min_unique_paths {
            return Err(
                "file_observation.enumerate.max_paths_per_set must be >= file_observation.enumerate.min_unique_paths"
                    .to_string(),
            );
        }
        if config.enabled && config.tty.enabled {
            if config
                .tty
                .operations
                .iter()
                .any(|operation| matches!(operation.as_str(), "read" | "readv"))
                && !config.collection.read.enabled()
            {
                return Err(
                    "file_observation.tty read operations require at least one file_observation.collection.read demand"
                        .to_string(),
                );
            }
            if config
                .tty
                .operations
                .iter()
                .any(|operation| matches!(operation.as_str(), "write" | "writev"))
                && !config.collection.write.enabled()
            {
                return Err(
                    "file_observation.tty write operations require at least one file_observation.collection.write demand"
                        .to_string(),
                );
            }
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
}

impl Default for FileTtyDocument {
    fn default() -> Self {
        Self {
            enabled: false,
            paths: ["/dev/tty", "/dev/pts/*", "/dev/ptmx"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            operations: ["read", "readv", "write", "writev"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            raw_event_retention: "summary".to_string(),
        }
    }
}

impl FileTtyDocument {
    pub(super) fn to_config(&self) -> Result<FileTtyObservationConfig, String> {
        if self.paths.iter().any(|path| path.is_empty()) {
            return Err("file_observation.tty.paths must not contain empty entries".to_string());
        }
        if self
            .operations
            .iter()
            .any(|operation| !matches!(operation.as_str(), "read" | "readv" | "write" | "writev"))
        {
            return Err(
                "file_observation.tty.operations accepts only read, readv, write and writev"
                    .to_string(),
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
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FileBulkReadDocument {
    pub enabled: bool,
    pub mode: String,
    pub raw_event_retention: String,
    pub max_paths_per_set: u32,
    pub path_set_chunk_max_paths: u32,
}

impl Default for FileBulkReadDocument {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "path_set".to_string(),
            raw_event_retention: "errors_only".to_string(),
            max_paths_per_set: 4096,
            path_set_chunk_max_paths: 256,
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
            max_paths_per_set: require_positive_u32(
                "file_observation.bulk_read.max_paths_per_set",
                self.max_paths_per_set,
            )?,
            path_set_chunk_max_paths: require_positive_u32(
                "file_observation.bulk_read.path_set_chunk_max_paths",
                self.path_set_chunk_max_paths,
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
            enabled: false,
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
