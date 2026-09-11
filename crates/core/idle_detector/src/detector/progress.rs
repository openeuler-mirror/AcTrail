//! Projects semantic progress into idle task state.

use std::collections::BTreeMap;
use std::time::SystemTime;

use idle_contract::{IdleInterval, IdleIntervalId, IdleStoreOp};
use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionStatus, attr_keys};

use super::IdleDetector;

impl IdleDetector {
    /// Applies progress to the task already attributed during batch processing.
    pub(super) fn on_update(&mut self, update: SemanticActionUpdate, task_id: Option<String>) {
        let Some(task_id) = task_id else {
            return;
        };
        let trace_id = update.action.trace_id;
        let task_key = (trace_id, task_id);

        let Some(task) = self.tasks.get_mut(&task_key) else {
            return;
        };

        // A terminal lifecycle report can race with the final semantic batch.
        // Keep accepting a late action while an interval is still open so its
        // end timestamp comes from the observed progress.
        if !task.active && task.open_interval.is_none() {
            return;
        }

        if task
            .action_revisions
            .get(&update.action.action_id)
            .is_some_and(|revision| *revision >= update.revision)
        {
            return;
        }

        task.action_revisions
            .insert(update.action.action_id.clone(), update.revision);

        task.last_progress_at = task.last_progress_at.max(update.observed_at);
        if let Some(end_time) = update.action.end_time {
            task.last_progress_at = task.last_progress_at.max(end_time);
        }

        let process_key = (trace_id, update.action.process);

        if update.action.kind == SemanticActionKind::AgentIdentity
            && update.action.status == SemanticActionStatus::Success
        {
            self.agent_processes.insert(process_key);
            // An identity may first be reported as InProgress, which uses the
            // action ID as its protection key. Success must clear that key as
            // well as the process-scoped key used by observed child actions.
            task.active_work_keys.remove(&update.action.action_id);
            task.active_work_keys
                .remove(&process_activity_key(&update.action));
        } else if is_observed_child_process_action(&update.action)
            || (is_agent_process_exec(&update.action)
                && self.agent_processes.contains(&process_key))
        {
            task.active_work_keys
                .insert(process_activity_key(&update.action));
        } else if update.action.kind == SemanticActionKind::ProcessExit {
            task.active_work_keys
                .remove(&process_activity_key(&update.action));
        } else if update.action.status == SemanticActionStatus::InProgress {
            task.active_work_keys
                .insert(update.action.action_id.clone());
        } else {
            task.active_work_keys.remove(&update.action.action_id);
        }

        let progress_at = update.action.end_time.unwrap_or(update.observed_at);
        if let Some(op) = task.close_interval(progress_at) {
            self.pending_ops.push(op);
        }
    }

    /// Advances the scheduler; called by the daemon each drain cycle.
    pub fn tick(&mut self, now: SystemTime) {
        let task_keys = self.tasks.keys().cloned().collect::<Vec<_>>();
        for task_key in task_keys {
            let Some(task) = self.tasks.get_mut(&task_key) else {
                continue;
            };
            if !task.active {
                if task
                    .completion_deadline
                    .is_some_and(|deadline| deadline <= now)
                {
                    if let Some(op) = task.close_interval(now) {
                        self.pending_ops.push(op);
                    }
                }
                continue;
            }
            if task.waiting_for_user || task.has_active_work() {
                continue;
            }
            let Some(deadline) = task.last_progress_at.checked_add(self.idle_threshold) else {
                continue;
            };
            if deadline >= now || task.open_interval.is_some() {
                continue;
            }
            let interval_id = self.next_interval_id;
            self.next_interval_id = self.next_interval_id.saturating_add(1);
            let interval = IdleInterval {
                id: IdleIntervalId::new(interval_id),
                trace_id: task.trace_id,
                task_id: task.task_id.clone(),
                // Per contract: interval starts at last_progress_at + threshold.
                start_time: deadline,
                end_time: None,
            };
            task.open_interval = Some(interval.clone());
            self.pending_ops.push(IdleStoreOp::UpsertInterval(interval));
        }
    }
}

fn is_observed_child_process_action(action: &SemanticAction) -> bool {
    matches!(
        action.kind,
        SemanticActionKind::ProcessExec | SemanticActionKind::CommandInvocation
    ) && action
        .attributes
        .get(attr_keys::process_parent::ID)
        .is_some_and(|_| {
            action
                .attributes
                .get(attr_keys::process_parent::IDENTITY_STATE)
                .is_some_and(|state| state == "observed")
        })
}

fn is_agent_process_exec(action: &SemanticAction) -> bool {
    matches!(
        action.kind,
        SemanticActionKind::ProcessExec | SemanticActionKind::CommandInvocation
    ) && !action
        .attributes
        .contains_key(attr_keys::process_parent::IDENTITY_STATE)
}

fn process_activity_key(action: &SemanticAction) -> String {
    format!("process:{}", action.process.get())
}

/// One observable semantic-action increment consumed by the idle state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SemanticActionUpdate {
    pub action: SemanticAction,
    pub observed_at: SystemTime,
    pub revision: u64,
}

/// Numeric attributes whose increase represents observable progress.
const PROGRESS_COUNTER_KEYS: &[&str] = &[
    attr_keys::sse::CONTENT_DELTA_COUNT,
    attr_keys::sse::REASONING_DELTA_COUNT,
    attr_keys::sse::TOOL_DELTA_COUNT,
    attr_keys::sse::EVENT_COUNT,
    attr_keys::llm_response::COMPLETION_TOKENS,
    attr_keys::llm_response::PROMPT_TOKENS,
    attr_keys::llm_response::TOTAL_TOKENS,
    attr_keys::llm_response::REASONING_TOKENS,
    attr_keys::mcp::MESSAGE_SEQUENCE,
    attr_keys::payload::SEQUENCE,
    attr_keys::file::WRITE_COUNT,
    attr_keys::file::BYTES_WRITTEN,
];

#[derive(Clone, Debug)]
struct ActionCheckpoint {
    revision: u64,
    status: SemanticActionStatus,
    end_time: Option<SystemTime>,
    counters: BTreeMap<&'static str, i64>,
}

/// Converts evolving semantic actions into idempotent progress updates.
#[derive(Clone, Debug, Default)]
pub(super) struct ProgressProjection {
    checkpoints: BTreeMap<(TraceId, String), ActionCheckpoint>,
}

impl ProgressProjection {
    pub(super) fn remove_trace_checkpoints(&mut self, trace_id: TraceId) {
        self.checkpoints
            .retain(|(checkpoint_trace_id, _), _| *checkpoint_trace_id != trace_id);
    }

    pub(super) fn project_batch<'a>(
        &mut self,
        actions: impl IntoIterator<Item = &'a SemanticAction>,
        observed_at: SystemTime,
        mut sink: impl FnMut(SemanticActionUpdate),
    ) {
        for action in actions {
            if !eligible_for_progress(action.kind) {
                continue;
            }
            let key = (action.trace_id, action.action_id.clone());
            let checkpoint = self.checkpoints.get(&key);
            if !Self::is_progress(checkpoint, action) {
                continue;
            }
            let revision = checkpoint.map_or(1, |prior| prior.revision.saturating_add(1));
            self.checkpoints.insert(
                key,
                ActionCheckpoint {
                    revision,
                    status: action.status,
                    end_time: action.end_time,
                    counters: Self::counters(action),
                },
            );
            sink(SemanticActionUpdate {
                action: action.clone(),
                observed_at,
                revision,
            });
        }
    }

    fn is_progress(previous: Option<&ActionCheckpoint>, next: &SemanticAction) -> bool {
        let Some(previous) = previous else {
            return true;
        };
        if next.kind == SemanticActionKind::ProcessExit {
            // A replayed exit may close activity reopened by a late exec update.
            return true;
        }
        if next.status != previous.status || end_time_advanced(previous.end_time, next.end_time) {
            return true;
        }
        let next_counters = Self::counters(next);
        PROGRESS_COUNTER_KEYS.iter().any(|key| {
            let prior = previous.counters.get(key).copied().unwrap_or(0);
            let current = next_counters.get(key).copied().unwrap_or(0);
            current > prior
        })
    }

    fn counters(action: &SemanticAction) -> BTreeMap<&'static str, i64> {
        PROGRESS_COUNTER_KEYS
            .iter()
            .filter_map(|key| {
                action
                    .attributes
                    .get(*key)
                    .and_then(|raw| raw.parse::<i64>().ok())
                    .map(|value| (*key, value))
            })
            .collect()
    }
}

fn eligible_for_progress(kind: SemanticActionKind) -> bool {
    matches!(
        kind,
        SemanticActionKind::LlmCall
            | SemanticActionKind::LlmRequest
            | SemanticActionKind::LlmResponse
            | SemanticActionKind::McpToolCall
            | SemanticActionKind::McpRequest
            | SemanticActionKind::McpResponse
            | SemanticActionKind::McpStdin
            | SemanticActionKind::McpStdout
            | SemanticActionKind::SseStream
            | SemanticActionKind::SseEvent
            | SemanticActionKind::CommandInvocation
            | SemanticActionKind::ProcessExec
            | SemanticActionKind::ProcessExit
            | SemanticActionKind::AgentInvocation
            | SemanticActionKind::AgentIdentity
            | SemanticActionKind::AgentExit
            | SemanticActionKind::FileWrite
            | SemanticActionKind::FileModify
    )
}

fn end_time_advanced(previous: Option<SystemTime>, next: Option<SystemTime>) -> bool {
    match (previous, next) {
        (None, Some(_)) => true,
        (Some(previous), Some(next)) => next > previous,
        _ => false,
    }
}
