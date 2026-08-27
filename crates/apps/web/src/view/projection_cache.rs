//! In-process cache for expensive action-tree projections.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use model_core::ids::TraceId;
use storage_core::StorageBackend;

use super::action_tree_projection::ActionDisplayProjection;

const CACHE_CAPACITY: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TraceRevision {
    action_count: u64,
    action_max_key: i64,
    link_count: u64,
    link_max_rowid: i64,
}

impl TraceRevision {
    fn load(storage: &dyn StorageBackend, trace_id: TraceId) -> Option<TraceRevision> {
        let revision = storage.semantic_action_trace_revision(trace_id).ok()?;
        Some(TraceRevision {
            action_count: revision.action_count,
            action_max_key: revision.action_max_key,
            link_count: revision.link_count,
            link_max_rowid: revision.link_max_rowid,
        })
    }
}

struct ProjectionCache {
    entries: HashMap<(String, u64), Arc<ActionDisplayProjection>>,
    order: VecDeque<(String, u64)>,
    trace_revisions: HashMap<(String, u64), TraceRevision>,
}

impl ProjectionCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            trace_revisions: HashMap::new(),
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
        self.trace_revisions.clear();
        count
    }

    fn clear_trace(&mut self, key: &(String, u64)) -> usize {
        let removed = self.entries.remove(key).is_some() as usize;
        self.order.retain(|entry| entry != key);
        removed
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

pub(super) fn sync_trace_revision(
    storage: &dyn StorageBackend,
    storage_path: &Path,
    trace_id: TraceId,
) {
    let Some(revision) = TraceRevision::load(storage, trace_id) else {
        return;
    };
    let key = (storage_key_string(storage_path), trace_id.get());
    if let Ok(mut cache) = cache_state().lock() {
        let stale = match cache.trace_revisions.get(&key) {
            Some(previous) => *previous != revision,
            None => {
                cache.trace_revisions.insert(key, revision);
                return;
            }
        };
        if stale {
            cache.clear_trace(&key);
            cache.trace_revisions.insert(key, revision);
        }
    }
}

pub fn clear_projection_cache() -> usize {
    cache_state()
        .lock()
        .map(|mut cache| cache.clear_all())
        .unwrap_or(0)
}

pub(super) fn cached_action_display_projection(
    storage_path: &Path,
    trace_id: TraceId,
    loader: impl FnOnce() -> Result<ActionDisplayProjection, String>,
) -> Result<Arc<ActionDisplayProjection>, String> {
    let key = (storage_key_string(storage_path), trace_id.get());
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
