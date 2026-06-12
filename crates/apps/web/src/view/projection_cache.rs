//! In-process cache for expensive action-tree projections.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use model_core::ids::TraceId;

use super::action_tree_projection::ActionDisplayProjection;

const CACHE_CAPACITY: usize = 8;

struct ProjectionCache {
    entries: HashMap<(String, u64), Arc<ActionDisplayProjection>>,
    order: VecDeque<(String, u64)>,
}

impl ProjectionCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
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
}

fn cache_state() -> &'static Mutex<ProjectionCache> {
    static CACHE: OnceLock<Mutex<ProjectionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ProjectionCache::new()))
}

fn cache_key(storage_path: &Path, trace_id: TraceId) -> (String, u64) {
    let path = storage_path
        .canonicalize()
        .unwrap_or_else(|_| storage_path.to_path_buf());
    (path.to_string_lossy().into_owned(), trace_id.get())
}

pub(super) fn cached_action_display_projection(
    storage_path: &Path,
    trace_id: TraceId,
    loader: impl FnOnce() -> Result<ActionDisplayProjection, String>,
) -> Result<Arc<ActionDisplayProjection>, String> {
    let key = cache_key(storage_path, trace_id);
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
