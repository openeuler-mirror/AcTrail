use model_core::ids::TraceId;
use std::collections::{BTreeMap, BTreeSet};
/// Per-cycle routing results; backend session state remains authoritative.
#[derive(Default)]
pub struct SamplingSelection {
    claimed: BTreeSet<TraceId>,
    fallback_metadata: BTreeMap<TraceId, BTreeMap<String, String>>,
}
impl SamplingSelection {
    pub fn claim(&mut self, id: TraceId) {
        self.claimed.insert(id);
    }
    pub fn is_claimed(&self, id: TraceId) -> bool {
        self.claimed.contains(&id)
    }
    pub fn fallback_metadata(&mut self, id: TraceId) -> &mut BTreeMap<String, String> {
        self.fallback_metadata.entry(id).or_default()
    }
    pub fn take_fallback_metadata(&mut self, id: TraceId) -> BTreeMap<String, String> {
        self.fallback_metadata.remove(&id).unwrap_or_default()
    }
}
