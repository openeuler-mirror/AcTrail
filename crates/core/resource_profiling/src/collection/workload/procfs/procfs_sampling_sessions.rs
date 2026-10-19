use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use crate::{
    ProcfsWorkloadProfilingConfig, ProcfsWorkloadResourceProfilingBackend, ProcfsWorkloadSample,
    VerifiedProcessTarget,
};
use model_core::{ids::TraceId, process::ProcessIdentity};

pub(crate) struct ProcfsSamplingSessions {
    config: ProcfsWorkloadProfilingConfig,
    backends: BTreeMap<TraceId, ProcfsWorkloadResourceProfilingBackend>,
}

impl ProcfsSamplingSessions {
    pub(crate) fn new(config: ProcfsWorkloadProfilingConfig) -> Self {
        Self {
            config,
            backends: BTreeMap::new(),
        }
    }

    pub(crate) fn include_children(&self) -> bool {
        self.config.include_children()
    }

    pub(crate) fn sample(
        &mut self,
        trace: TraceId,
        root: ProcessIdentity,
        targets: &[VerifiedProcessTarget],
        sampled_at: Instant,
    ) -> Result<Option<ProcfsWorkloadSample>, String> {
        let backend = match self.backends.entry(trace) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert(
                ProcfsWorkloadResourceProfilingBackend::new(root, self.config)?,
            ),
        };
        backend.sample(targets, sampled_at)
    }

    pub(crate) fn retain(&mut self, active: &BTreeSet<TraceId>) {
        self.backends.retain(|trace, _| active.contains(trace));
    }
}
