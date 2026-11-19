use std::collections::HashMap;

use super::TerminalActionLimits;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalActionCapacity {
    PerTrace,
    Global,
    KeyBytes,
}

impl TerminalActionCapacity {
    pub fn code(self) -> &'static str {
        match self {
            Self::PerTrace => "terminal_action_per_trace_capacity",
            Self::Global => "terminal_action_global_capacity",
            Self::KeyBytes => "terminal_action_key_byte_capacity",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalActionRejection<R> {
    Duplicate,
    Conflict { previous: R },
    Capacity(TerminalActionCapacity),
}

pub(super) struct TerminalActionLedger<R> {
    limits: TerminalActionLimits,
    by_trace: HashMap<u64, TraceEntries<R>>,
    entries: usize,
    key_bytes: usize,
}

pub(super) struct TraceEntries<R> {
    revisions: HashMap<String, R>,
    key_bytes: usize,
}

impl<R: Copy + Eq> TerminalActionLedger<R> {
    pub(super) fn new(limits: TerminalActionLimits) -> Self {
        Self {
            limits,
            by_trace: HashMap::new(),
            entries: 0,
            key_bytes: 0,
        }
    }

    pub(super) fn rejection(
        &self,
        trace_id: u64,
        id: &str,
        revision: R,
    ) -> Option<TerminalActionRejection<R>> {
        let trace = self.by_trace.get(&trace_id);
        if let Some(previous) = trace.and_then(|trace| trace.revisions.get(id)).copied() {
            return Some(if previous == revision {
                TerminalActionRejection::Duplicate
            } else {
                TerminalActionRejection::Conflict { previous }
            });
        }
        let capacity = if trace.is_some_and(|trace| trace.revisions.len() >= self.limits.per_trace)
        {
            TerminalActionCapacity::PerTrace
        } else if self.entries >= self.limits.entries {
            TerminalActionCapacity::Global
        } else if id.len() > self.limits.key_bytes.saturating_sub(self.key_bytes) {
            TerminalActionCapacity::KeyBytes
        } else {
            return None;
        };
        Some(TerminalActionRejection::Capacity(capacity))
    }

    // Called only after an atomic successful admission of a previously unseen ID.
    pub(super) fn record(&mut self, trace_id: u64, id: String, revision: R) {
        let bytes = id.len();
        let trace = self
            .by_trace
            .entry(trace_id)
            .or_insert_with(|| TraceEntries {
                revisions: HashMap::new(),
                key_bytes: 0,
            });
        trace.revisions.insert(id, revision);
        trace.key_bytes += bytes;
        self.entries += 1;
        self.key_bytes += bytes;
    }

    pub(super) fn finish_trace(&mut self, trace_id: u64) -> Option<TraceEntries<R>> {
        let trace = self.by_trace.remove(&trace_id)?;
        self.entries -= trace.revisions.len();
        self.key_bytes -= trace.key_bytes;
        Some(trace)
    }

    pub(super) fn clear(&mut self) -> HashMap<u64, TraceEntries<R>> {
        self.entries = 0;
        self.key_bytes = 0;
        std::mem::take(&mut self.by_trace)
    }
}
