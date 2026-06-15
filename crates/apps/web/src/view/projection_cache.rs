//! In-process cache for expensive action-tree projections.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use model_core::ids::TraceId;

use super::action_tree_projection::ActionDisplayProjection;

const CACHE_CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StorageRevision {
    modified_secs: u64,
    len: u64,
}

struct ProjectionCache {
    entries: HashMap<(String, u64), Arc<ActionDisplayProjection>>,
    order: VecDeque<(String, u64)>,
    storage_revisions: HashMap<String, StorageRevision>,
}

impl ProjectionCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            storage_revisions: HashMap::new(),
        }
    }

    fn get(&mut self, key: &(String, u64)) -> Option<Arc<ActionDisplayProjection>> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: (String, u64), value: Arc<ActionDisplayProjection>) {
        if self.entries.contains_key(&key) {
            self.order.retain(|entry| entry != &key);
        } else if self.entries.len() >= CACHE_CAPACITY {
            if let Some(evicted) = self.order.pop_front() {
                self.entries.remove(&evicted);
            }
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    fn clear_all(&mut self) -> usize {
        let count = self.entries.len();
        self.entries.clear();
        self.order.clear();
        self.storage_revisions.clear();
        count
    }

    fn clear_storage(&mut self, storage_key: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(|(path, _), _| path != storage_key);
        self.order.retain(|(path, _)| path != storage_key);
        self.storage_revisions.remove(storage_key);
        before - self.entries.len()
    }

    fn sync_storage_revision(&mut self, storage_path: &Path) {
        let storage_key = storage_key_string(storage_path);
        let revision = read_storage_revision(storage_path);
        let stale = match (revision, self.storage_revisions.get(&storage_key)) {
            (None, None) => false,
            (None, Some(_)) => true,
            (Some(current), Some(previous)) => current != *previous,
            (Some(current), None) => {
                self.storage_revisions.insert(storage_key.clone(), current);
                return;
            }
        };
        if stale {
            self.clear_storage(&storage_key);
            if let Some(current) = revision {
                self.storage_revisions.insert(storage_key, current);
            }
        }
    }
}

fn cache_state() -> &'static Mutex<ProjectionCache> {
    static CACHE: OnceLock<Mutex<ProjectionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ProjectionCache::new()))
}

fn storage_key_string(storage_path: &Path) -> String {
    storage_path
        .canonicalize()
        .unwrap_or_else(|_| storage_path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn read_storage_revision(storage_path: &Path) -> Option<StorageRevision> {
    let metadata = std::fs::metadata(storage_path).ok()?;
    let modified_secs = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(StorageRevision {
        modified_secs,
        len: metadata.len(),
    })
}

fn sync_storage_revision(storage_path: &Path) {
    if let Ok(mut cache) = cache_state().lock() {
        cache.sync_storage_revision(storage_path);
    }
}

pub fn clear_projection_cache() -> usize {
    cache_state()
        .lock()
        .map(|mut cache| cache.clear_all())
        .unwrap_or(0)
}

pub fn clear_projection_cache_json() -> String {
    let cleared = clear_projection_cache();
    format!("{{\"cleared\":{cleared}}}")
}

pub(super) fn cached_action_display_projection(
    storage_path: &Path,
    trace_id: TraceId,
    loader: impl FnOnce() -> Result<ActionDisplayProjection, String>,
) -> Result<Arc<ActionDisplayProjection>, String> {
    sync_storage_revision(storage_path);
    let key = (
        storage_key_string(storage_path),
        trace_id.get(),
    );
    if let Ok(mut cache) = cache_state().lock() {
        if let Some(projection) = cache.get(&key) {
            return Ok(projection);
        }
    }
    let projection = Arc::new(loader()?);
    if let Ok(mut cache) = cache_state().lock() {
        cache.insert(key, Arc::clone(&projection));
    }
    Ok(projection)
}
