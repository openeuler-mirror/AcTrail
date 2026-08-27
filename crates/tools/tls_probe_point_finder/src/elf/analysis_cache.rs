//! Path-independent caches for expensive ELF symbol and pattern analysis.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;

use crate::binary_identity::BinaryIdentity;
use crate::{ToolError, ToolResult};

use super::scan::PatternScanCache;
use super::symbols::SymbolScanCache;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BinaryAnalysisCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entries: usize,
}

pub struct BinaryAnalysisCache {
    capacity: usize,
    inner: RefCell<BinaryAnalysisCacheInner>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct BinaryAnalysisKey {
    identity: BinaryIdentity,
    file_generation: Option<BinaryFileGeneration>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct BinaryFileGeneration {
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanos: i64,
    changed_seconds: i64,
    changed_nanos: i64,
}

#[derive(Default)]
struct BinaryAnalysisCacheInner {
    next_access: u64,
    records: BTreeMap<BinaryAnalysisKey, (BinaryAnalysisRecord, u64)>,
    accesses: BTreeMap<u64, BinaryAnalysisKey>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

struct BinaryAnalysisRecord {
    patterns: PatternScanCache,
    symbols: SymbolScanCache,
    markers: BTreeMap<Vec<u8>, bool>,
    named_addresses: BTreeMap<String, Option<BTreeMap<String, u64>>>,
}

impl BinaryAnalysisCache {
    pub fn new(capacity: usize) -> ToolResult<Self> {
        if capacity == 0 {
            return Err(ToolError::new(
                "binary analysis cache capacity must be greater than zero",
            ));
        }
        Ok(Self {
            capacity,
            inner: RefCell::new(BinaryAnalysisCacheInner::default()),
        })
    }

    pub fn stats(&self) -> BinaryAnalysisCacheStats {
        let inner = self.inner.borrow();
        BinaryAnalysisCacheStats {
            hits: inner.hits,
            misses: inner.misses,
            evictions: inner.evictions,
            entries: inner.records.len(),
        }
    }

    pub(super) fn with_patterns<R>(
        &self,
        key: &BinaryAnalysisKey,
        operation: impl FnOnce(&mut PatternScanCache) -> R,
    ) -> R {
        self.with_record(key, |record| operation(&mut record.patterns))
    }

    pub(super) fn with_symbols<R>(
        &self,
        key: &BinaryAnalysisKey,
        operation: impl FnOnce(&mut SymbolScanCache) -> R,
    ) -> R {
        self.with_record(key, |record| operation(&mut record.symbols))
    }

    pub(super) fn with_markers<R>(
        &self,
        key: &BinaryAnalysisKey,
        operation: impl FnOnce(&mut BTreeMap<Vec<u8>, bool>) -> R,
    ) -> R {
        self.with_record(key, |record| operation(&mut record.markers))
    }

    pub(super) fn with_named_addresses<R>(
        &self,
        key: &BinaryAnalysisKey,
        operation: impl FnOnce(&mut BTreeMap<String, Option<BTreeMap<String, u64>>>) -> R,
    ) -> R {
        self.with_record(key, |record| operation(&mut record.named_addresses))
    }

    pub(super) fn record_analysis(&self, reused: bool) {
        let mut inner = self.inner.borrow_mut();
        if reused {
            inner.hits = inner.hits.saturating_add(1);
        } else {
            inner.misses = inner.misses.saturating_add(1);
        }
    }

    fn with_record<R>(
        &self,
        key: &BinaryAnalysisKey,
        operation: impl FnOnce(&mut BinaryAnalysisRecord) -> R,
    ) -> R {
        let mut inner = self.inner.borrow_mut();
        inner.ensure_access_space();
        if let Some(previous_access) = inner.records.get(key).map(|(_, access)| *access) {
            inner.accesses.remove(&previous_access);
        } else {
            while inner.records.len() >= self.capacity {
                inner.evict_oldest();
            }
            inner
                .records
                .insert(key.clone(), (BinaryAnalysisRecord::default(), 0));
        }
        let access = inner.take_access_id();
        let (record, record_access) = inner
            .records
            .get_mut(key)
            .expect("binary analysis record is present");
        *record_access = access;
        let result = operation(record);
        inner.accesses.insert(access, key.clone());
        result
    }
}

impl BinaryAnalysisKey {
    pub(super) fn new(identity: BinaryIdentity, metadata: &Metadata) -> Self {
        let file_generation = (identity.identity_type_code
            == crate::binary_identity::BinaryIdentityTypeCode::ElfExecutableSampleSha256V1)
            .then(|| BinaryFileGeneration::from_metadata(metadata));
        Self {
            identity,
            file_generation,
        }
    }
}

impl BinaryFileGeneration {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanos: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanos: metadata.ctime_nsec(),
        }
    }
}

impl BinaryAnalysisCacheInner {
    fn take_access_id(&mut self) -> u64 {
        let access = self.next_access;
        self.next_access += 1;
        access
    }

    fn ensure_access_space(&mut self) {
        if self.next_access == u64::MAX {
            let removed = u64::try_from(self.records.len()).unwrap_or(u64::MAX);
            self.records.clear();
            self.accesses.clear();
            self.evictions = self.evictions.saturating_add(removed);
            self.next_access = 0;
        }
    }

    fn evict_oldest(&mut self) {
        let Some((&access, key)) = self.accesses.first_key_value() else {
            self.records.clear();
            return;
        };
        let key = key.clone();
        self.accesses.remove(&access);
        if self.records.remove(&key).is_some() {
            self.evictions = self.evictions.saturating_add(1);
        }
    }
}

impl Default for BinaryAnalysisRecord {
    fn default() -> Self {
        Self {
            patterns: PatternScanCache::default(),
            symbols: SymbolScanCache::new(),
            markers: BTreeMap::new(),
            named_addresses: BTreeMap::new(),
        }
    }
}
