//! Traces another daemon service watches, and the LLM requests collected for it while live
//! events are processed.

use std::collections::BTreeSet;

use model_core::ids::TraceId;

/// An LLM request of a watched Trace; its response waits until `hold` is released.
pub struct HeldLlmRequest {
    pub(crate) hold: u64,
    pub(crate) trace_id: TraceId,
    /// The `llm.request` action the payload pipeline recognized.
    pub(crate) request_action_id: String,
}

/// Keeps the LLM requests of watched Traces until they are taken after the drain.
#[derive(Default)]
pub(crate) struct WatchedTraces {
    traces: BTreeSet<TraceId>,
    llm_requests: Vec<HeldLlmRequest>,
}

impl WatchedTraces {
    pub(crate) fn watch(&mut self, trace_id: TraceId, watch: bool) {
        if watch {
            self.traces.insert(trace_id);
        } else {
            self.traces.remove(&trace_id);
        }
    }

    pub(crate) fn contains(&self, trace_id: TraceId) -> bool {
        self.traces.contains(&trace_id)
    }

    /// True while no Trace is watched, which only checkpoints do; live event processing then
    /// skips the work it does for watched Traces.
    pub(crate) fn is_empty(&self) -> bool {
        self.traces.is_empty()
    }

    pub(crate) fn push_llm_request(&mut self, request: HeldLlmRequest) {
        self.llm_requests.push(request);
    }

    pub(crate) fn take_llm_requests(&mut self) -> Vec<HeldLlmRequest> {
        std::mem::take(&mut self.llm_requests)
    }
}
