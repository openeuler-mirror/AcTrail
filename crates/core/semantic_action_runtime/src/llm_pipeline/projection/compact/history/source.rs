use std::sync::Arc;

use super::super::observation::CompactLimitExceeded;
use super::super::state::RequestHistoryWitness;
use super::{ContextHistory, HistoryContinuationFilter, HistoryObservation};

/// Attempts share immutable observations through the request's pooled witness.
/// This coordinator only holds per-attempt limit reporting.
pub(in crate::llm_pipeline::projection::compact) struct HistoryMaterializer<'a> {
    filter: &'a HistoryContinuationFilter,
    limit_exceeded: Option<CompactLimitExceeded>,
}

impl<'a> HistoryMaterializer<'a> {
    pub(in crate::llm_pipeline::projection::compact) fn new(
        filter: &'a HistoryContinuationFilter,
    ) -> Self {
        Self {
            filter,
            limit_exceeded: None,
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn histories(
        &mut self,
        implementation: &Arc<RequestHistoryWitness>,
        adoption: &Arc<RequestHistoryWitness>,
    ) -> Option<[ContextHistory; 2]> {
        Some([self.snapshot(implementation)?, self.snapshot(adoption)?])
    }

    /// Called only after a unique compaction-to-adoption winner, whose compaction candidate history is cached.
    /// Missing or over-budget A evidence only omits the optional source link.
    pub(in crate::llm_pipeline::projection::compact) fn consumes_source(
        &mut self,
        source: &Arc<RequestHistoryWitness>,
        implementation: &Arc<RequestHistoryWitness>,
    ) -> bool {
        self.histories(source, implementation)
            .is_some_and(|[source, implementation]| {
                self.filter.consumes_history(&source, &implementation)
            })
    }

    fn snapshot(&mut self, source: &Arc<RequestHistoryWitness>) -> Option<ContextHistory> {
        let observation = source
            .snapshot
            .get_or_init(|| self.filter.snapshot(source.source.as_ref()));
        if let HistoryObservation::LimitExceeded(limit) = observation {
            self.limit_exceeded = Some(*limit);
        }
        match observation {
            HistoryObservation::Present(history) => Some(history.clone()),
            HistoryObservation::Absent | HistoryObservation::LimitExceeded(_) => None,
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn take_limit_exceeded(
        &mut self,
    ) -> Option<CompactLimitExceeded> {
        self.limit_exceeded.take()
    }
}
