//! Bounded pairing of eBPF exec attempts with sched/exit results.

use std::collections::{BTreeMap, VecDeque};

use model_core::ids::TraceId;

use crate::loader::{
    KernelProcessExecArgEvent, KernelProcessExecAttemptEvent, KernelProcessExecResultEvent,
};

const PROCESS_EXEC_ARGV_COMPLETE: u32 = 16;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ExecAttemptKey {
    trace_id: TraceId,
    host_pid: u32,
    attempt_id: u64,
}

pub(super) struct ProcessExecAssembler {
    pending: BTreeMap<ExecAttemptKey, KernelProcessExecAttemptEvent>,
    order: VecDeque<ExecAttemptKey>,
    max_entries: usize,
    evictions: u64,
    orphan_args: u64,
}

impl ProcessExecAssembler {
    pub(super) fn new(max_entries: u32) -> Self {
        Self {
            pending: BTreeMap::new(),
            order: VecDeque::new(),
            max_entries: usize::try_from(max_entries)
                .expect("process pending entry limit must fit usize")
                .max(1),
            evictions: 0,
            orphan_args: 0,
        }
    }

    pub(super) fn observe_arg(&mut self, event: KernelProcessExecArgEvent) {
        let key = ExecAttemptKey {
            trace_id: event.trace_id,
            host_pid: event.host_pid,
            attempt_id: event.attempt_id,
        };
        let Some(attempt) = self.pending.get_mut(&key) else {
            self.orphan_args = self.orphan_args.saturating_add(1);
            return;
        };
        attempt.capture_flags |= event.capture_flags & !PROCESS_EXEC_ARGV_COMPLETE;
        if event.capture_flags & PROCESS_EXEC_ARGV_COMPLETE != 0 {
            return;
        }
        let Ok(expected_index) = u32::try_from(attempt.argv.len()) else {
            self.orphan_args = self.orphan_args.saturating_add(1);
            return;
        };
        if event.index != expected_index {
            self.orphan_args = self.orphan_args.saturating_add(1);
            return;
        }
        if event.arg.is_empty() && event.capture_flags != 0 {
            return;
        }
        attempt.argv.push(event.arg);
    }

    pub(super) fn observe_attempt(&mut self, event: KernelProcessExecAttemptEvent) {
        let key = ExecAttemptKey {
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

    pub(super) fn take_success(
        &mut self,
        trace_id: TraceId,
        kernel_tgid: u32,
        attempt_id: u64,
    ) -> Option<KernelProcessExecAttemptEvent> {
        self.take(ExecAttemptKey {
            trace_id,
            host_pid: kernel_tgid,
            attempt_id,
        })
    }

    pub(super) fn take_failure(
        &mut self,
        event: &KernelProcessExecResultEvent,
    ) -> Option<KernelProcessExecAttemptEvent> {
        self.take(ExecAttemptKey {
            trace_id: event.trace_id,
            host_pid: event.host_pid,
            attempt_id: event.attempt_id,
        })
    }

    fn take(&mut self, key: ExecAttemptKey) -> Option<KernelProcessExecAttemptEvent> {
        if key.attempt_id == 0 {
            return None;
        }
        let event = self.pending.remove(&key)?;
        if let Some(position) = self.order.iter().position(|candidate| *candidate == key) {
            self.order.remove(position);
        }
        Some(event)
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.pending.retain(|key, _| key.trace_id != trace_id);
        self.order.retain(|key| key.trace_id != trace_id);
    }

    pub(super) fn evictions(&self) -> u64 {
        self.evictions
    }

    pub(super) fn orphan_args(&self) -> u64 {
        self.orphan_args
    }
}
