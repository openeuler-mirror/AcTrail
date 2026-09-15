//! Bounded pairing of eBPF fork/clone attempts with syscall results.

use std::collections::{BTreeMap, VecDeque};

use model_core::ids::TraceId;

use crate::loader::{KernelProcessForkAttemptEvent, KernelProcessForkResultEvent};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ForkAttemptKey {
    trace_id: TraceId,
    host_pid: u32,
    attempt_id: u64,
}

pub(super) struct ProcessForkAssembler {
    pending: BTreeMap<ForkAttemptKey, KernelProcessForkAttemptEvent>,
    order: VecDeque<ForkAttemptKey>,
    max_entries: usize,
    evictions: u64,
}

impl ProcessForkAssembler {
    pub(super) fn new(max_entries: u32) -> Self {
        Self {
            pending: BTreeMap::new(),
            order: VecDeque::new(),
            max_entries: usize::try_from(max_entries)
                .expect("process pending entry limit must fit usize")
                .max(1),
            evictions: 0,
        }
    }

    pub(super) fn observe_attempt(&mut self, event: KernelProcessForkAttemptEvent) {
        let key = ForkAttemptKey {
            trace_id: event.trace_id,
            host_pid: event.host_pid,
            attempt_id: event.attempt_id,
        };
        if self.pending.contains_key(&key) {
            self.pending.insert(key, event);
            return;
        }
        while self.pending.len() >= self.max_entries {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if self.pending.remove(&oldest).is_some() {
                self.evictions = self.evictions.saturating_add(1);
            }
        }
        self.pending.insert(key, event);
        self.order.push_back(key);
    }

    pub(super) fn take_result(
        &mut self,
        event: &KernelProcessForkResultEvent,
    ) -> Option<KernelProcessForkAttemptEvent> {
        let key = ForkAttemptKey {
            trace_id: event.trace_id,
            host_pid: event.host_pid,
            attempt_id: event.attempt_id,
        };
        let attempt = self.pending.remove(&key)?;
        if let Some(position) = self.order.iter().position(|candidate| *candidate == key) {
            self.order.remove(position);
        }
        Some(attempt)
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.pending.retain(|key, _| key.trace_id != trace_id);
        self.order.retain(|key| key.trace_id != trace_id);
    }

    pub(super) fn evictions(&self) -> u64 {
        self.evictions
    }
}
