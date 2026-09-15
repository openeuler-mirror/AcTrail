//! Applies turn and user-interaction lifecycle events.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use idle_contract::{
    TurnLifecycleEvent, TurnLifecycleKind, UserInteractionEvent, UserInteractionState,
};
use model_core::ids::TraceId;

use super::{FINAL_ACTION_GRACE, IdleDetector};
use crate::task_state::TaskIdleState;

impl IdleDetector {
    pub fn on_turn_lifecycle(&mut self, event: TurnLifecycleEvent) -> bool {
        if event.validate().is_err() {
            return false;
        }
        match event.kind {
            TurnLifecycleKind::Started => {
                let task_key = (event.trace_id, event.task_id.clone()); // Look up by trace and task ID.
                if self.tasks.get(&task_key).is_some_and(|task| task.active) {
                    return true;
                }
                if self.tasks.contains_key(&task_key) {
                    self.close_task(task_key.clone(), event.observed_at);
                    self.tasks.remove(&task_key);
                }
                self.open_task(event.trace_id, event.task_id, event.observed_at);
                true
            }
            TurnLifecycleKind::Completed => {
                let task_key = (event.trace_id, event.task_id.clone());
                let Some(task) = self.tasks.get(&task_key) else {
                    return false;
                };
                if !task.active {
                    return true;
                }
                self.deactivate_task(task_key, event.observed_at);
                true
            }
        }
    }

    pub fn on_user_interaction(&mut self, event: UserInteractionEvent) -> bool {
        if event.validate().is_err() {
            return false;
        }
        let task_key = (event.trace_id, event.task_id.clone());
        if !self.tasks.get(&task_key).is_some_and(|task| task.active) {
            return false;
        }
        let task = self
            .tasks
            .get_mut(&task_key)
            .expect("validated active task must still exist");
        match event.state {
            UserInteractionState::Requested => {
                if task.pending_interaction_ids.insert(event.interaction_id) {
                    if let Some(op) = task.close_interval(event.observed_at) {
                        self.pending_ops.push(op);
                    }
                    task.waiting_for_user = true;
                }
            }
            UserInteractionState::Resolved => {
                if task.pending_interaction_ids.remove(&event.interaction_id)
                    && task.pending_interaction_ids.is_empty()
                {
                    task.waiting_for_user = false;
                    task.last_progress_at = task.last_progress_at.max(event.observed_at);
                }
            }
        }
        true
    }

    /// Hard-closes all tasks and open idle intervals when a trace reaches a terminal boundary.
    pub fn on_trace_ended(&mut self, trace_id: TraceId, now: SystemTime) {
        self.close_all_tasks_for_trace(trace_id, now);
    }

    /// Earliest deadline that still needs a scheduler wakeup.
    pub fn next_deadline(&self) -> Option<SystemTime> {
        self.tasks
            .values()
            .filter_map(|task| task.next_deadline(self.idle_threshold))
            .min()
    }

    /// Discards in-memory detector state for a trace after its intervals are finalized.
    pub fn forget_trace(&mut self, trace_id: TraceId) {
        self.tasks
            .retain(|(task_trace_id, _), _| *task_trace_id != trace_id);
        self.agent_processes
            .retain(|(task_trace_id, _)| *task_trace_id != trace_id);
        self.progress_projection.remove_trace_checkpoints(trace_id);
    }

    // Internal task-state transitions.
    pub(super) fn open_task(&mut self, trace_id: TraceId, task_id: String, now: SystemTime) {
        let task_key = (trace_id, task_id.clone());
        if self.tasks.contains_key(&task_key) {
            return;
        }
        let task = TaskIdleState {
            task_id,
            trace_id,
            active: true,
            waiting_for_user: false,
            pending_interaction_ids: BTreeSet::new(),
            action_revisions: BTreeMap::new(),
            last_progress_at: now,
            active_work_keys: BTreeSet::new(),
            open_interval: None,
            completion_deadline: None,
        };
        self.tasks.insert(task_key.clone(), task);
    }

    /// Hard-closes every task in a trace at a trace-level boundary.
    pub(super) fn close_all_tasks_for_trace(&mut self, trace_id: TraceId, now: SystemTime) {
        let task_keys = self
            .tasks
            .keys()
            .filter(|(task_trace_id, _)| *task_trace_id == trace_id)
            .cloned()
            .collect::<Vec<_>>();
        for task_key in task_keys {
            self.close_task(task_key, now);
        }
    }

    /// Hard-closes one task immediately, including any open idle interval.
    pub(super) fn close_task(&mut self, task_key: (TraceId, String), now: SystemTime) {
        let Some(task) = self.tasks.get_mut(&task_key) else {
            return;
        };
        if !task.active {
            // A completed task may retain an interval briefly for a racing final semantic action. A hard boundary must finalize it.
            if let Some(op) = task.close_interval(now) {
                self.pending_ops.push(op);
            }
            return;
        }
        if let Some(op) = task.close_interval(now) {
            self.pending_ops.push(op);
        }
        task.deactivate_activity();
    }

    /// Soft-completes a task, preserving an open interval for a racing final action.
    pub(super) fn deactivate_task(
        &mut self,
        task_key: (TraceId, String),
        completed_at: SystemTime,
    ) {
        let Some(task) = self.tasks.get_mut(&task_key) else {
            return;
        };
        if !task.active {
            return;
        }
        task.deactivate_activity();
        task.completion_deadline = task
            .open_interval
            .as_ref()
            .and_then(|_| completed_at.checked_add(FINAL_ACTION_GRACE));
    }
}
