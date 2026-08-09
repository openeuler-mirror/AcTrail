//! Binary TLS probe-plan storage used by the sync resolver.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tls_probe_point_finder::BinaryIdentity;
use tls_probe_point_finder::fast::ProbeConsumer;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct BinaryPlanKey {
    path: PathBuf,
    len: u64,
    modified: Option<(u64, u32)>,
    identity: BinaryIdentity,
    consumer: ProbeConsumer,
}

#[derive(Clone, Debug)]
pub(super) enum BinaryPlanRecord {
    Found(Vec<BinaryPlanDescriptor>),
    Unsupported(String),
}

#[derive(Clone, Debug)]
pub(super) struct BinaryPlanDescriptor {
    pub(super) binary: PathBuf,
    pub(super) target_identity: BinaryIdentity,
    pub(super) binary_identity: BinaryIdentity,
    pub(super) provider: String,
    pub(super) source: String,
    pub(super) points: String,
}

pub(super) trait BinaryPlanStore {
    fn get(&self, key: &BinaryPlanKey) -> Result<Option<BinaryPlanRecord>, String>;
    fn put(&mut self, key: BinaryPlanKey, record: BinaryPlanRecord) -> Result<(), String>;
}

#[derive(Default)]
pub(super) struct InMemoryBinaryPlanStore {
    records: BTreeMap<BinaryPlanKey, BinaryPlanRecord>,
}

impl BinaryPlanKey {
    pub(super) fn for_path(path: &Path, consumer: ProbeConsumer) -> Result<Self, String> {
        let path = cache_path(path);
        let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        let identity =
            tls_probe_point_finder::elf_identity(&path).map_err(|error| error.to_string())?;
        Ok(Self {
            path,
            len: metadata.len(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| (duration.as_secs(), duration.subsec_nanos())),
            identity,
            consumer,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl BinaryPlanStore for InMemoryBinaryPlanStore {
    fn get(&self, key: &BinaryPlanKey) -> Result<Option<BinaryPlanRecord>, String> {
        Ok(self.records.get(key).cloned())
    }

    fn put(&mut self, key: BinaryPlanKey, record: BinaryPlanRecord) -> Result<(), String> {
        self.records.insert(key, record);
        Ok(())
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn cache_path(path: &Path) -> PathBuf {
    if is_proc_namespace_path(path) {
        path.to_path_buf()
    } else {
        canonical(path)
    }
}

fn is_proc_namespace_path(path: &Path) -> bool {
    let raw = path.as_os_str().to_string_lossy();
    raw.starts_with("/proc/") && (raw.contains("/root/") || raw.contains("/fd/"))
}
