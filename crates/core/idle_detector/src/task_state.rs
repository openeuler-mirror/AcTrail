//! Per-task idle state machine state.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

use idle_contract::{IdleInterval, IdleStoreOp};
use model_core::ids::TraceId;

#[derive(Clone, Debug)]
pub(crate) struct TaskIdleState {
    pub task_id: String,
    pub trace_id: TraceId,
    pub active: bool,
    pub waiting_for_user: bool,
    pub pending_interaction_ids: BTreeSet<String>,
    /// Highest real-time revision consumed for each action. This keeps the
    /// detector idempotent even when its sink receives a duplicate delivery.
    pub action_revisions: BTreeMap<String, u64>,
    pub last_progress_at: SystemTime,
    /// Actions still in flight (InProgress). Non-empty means the task chain is
    /// active and must never open an idle interval.
    pub active_work_keys: BTreeSet<String>,
    pub open_interval: Option<IdleInterval>,
    pub completion_deadline: Option<SystemTime>,
}

impl TaskIdleState {
    /// Active-child protection: any unfinished action keeps the task alive.
    pub fn has_active_work(&self) -> bool {
        self.active && !self.active_work_keys.is_empty()
    }

    pub fn next_deadline(&self, idle_threshold: Duration) -> Option<SystemTime> {
        if !self.active {
            return self.completion_deadline;
        }
        if self.waiting_for_user || self.has_active_work() {
            return None;
        }
        self.last_progress_at.checked_add(idle_threshold)
    }

    pub fn close_interval(&mut self, end_time: SystemTime) -> Option<IdleStoreOp> {
        let operation = self
            .open_interval
            .take()
            .map(|interval| finish_interval(interval, end_time));
        self.completion_deadline = None;
        operation
    }

    /// Ends task activity while leaving interval-closing policy to the caller.
    pub(super) fn deactivate_activity(&mut self) {
        self.active = false;
        self.waiting_for_user = false;
        self.pending_interaction_ids.clear();
        self.active_work_keys.clear();
        self.action_revisions.clear();
    }
}

pub(super) fn finish_interval(mut interval: IdleInterval, end_time: SystemTime) -> IdleStoreOp {
    if end_time > interval.start_time {
        interval.end_time = Some(end_time);
        IdleStoreOp::UpsertInterval(interval)
    } else {
        IdleStoreOp::DeleteInterval(interval.id)
    }
}
