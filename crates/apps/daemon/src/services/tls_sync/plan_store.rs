//! Binary TLS probe-plan storage used by the sync resolver.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tls_payload_sync::RuntimePlanDescriptor;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct BinaryPlanKey {
    path: PathBuf,
    len: u64,
    modified: Option<(u64, u32)>,
    build_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) enum BinaryPlanRecord {
    Found(RuntimePlanDescriptor),
    Unsupported(String),
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
    pub(super) fn for_path(path: &Path) -> std::io::Result<Self> {
        let path = canonical(path);
        let metadata = std::fs::metadata(&path)?;
        let build_id = tls_probe_point_finder::elf_build_id(&path).ok().flatten();
        Ok(Self {
            path,
            len: metadata.len(),
            modified: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| (duration.as_secs(), duration.subsec_nanos())),
            build_id,
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
