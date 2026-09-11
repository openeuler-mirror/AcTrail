//! Attributes semantic actions to active idle tasks.

use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use semantic_action::{SemanticAction, SemanticActionLink, attr_keys};

use super::IdleDetector;

impl IdleDetector {
    /// Resolves and records task ownership for every action in one batch.
    pub(super) fn attribute_batch_actions(
        &self,
        actions: &mut [SemanticAction],
        links: &[SemanticActionLink],
    ) -> BTreeSet<TraceId> {
        // Propagate task IDs only within the current batch.
        let action_keys = actions
            .iter()
            .map(|action| (action.trace_id, action.action_id.clone()))
            .collect::<BTreeSet<_>>();

        // Seed the mapping from action task IDs.
        let mut task_by_action = actions
            .iter()
            .filter_map(|action| {
                let task_id = self.resolve_task_id(action.trace_id, &action.attributes)?;
                Some(((action.trace_id, action.action_id.clone()), task_id))
            })
            .collect::<BTreeMap<_, _>>();

        // Propagate task IDs along in-batch lineage.
        for _ in 0..=links.len() {
            let mut changed = false;
            for link in links.iter().filter(|link| link.valid) {
                let parent_key = (link.trace_id, link.parent_action_id.clone());
                let child_key = (link.trace_id, link.child_action_id.clone());
                // Parent and child must share this trace and batch.
                if !action_keys.contains(&parent_key) || !action_keys.contains(&child_key) {
                    continue;
                }
                match (
                    task_by_action.get(&parent_key).cloned(),
                    task_by_action.get(&child_key).cloned(),
                ) {
                    (Some(task_id), None) => {
                        task_by_action.insert(child_key, task_id);
                        changed = true;
                    }
                    (None, Some(task_id)) => {
                        task_by_action.insert(parent_key, task_id);
                        changed = true;
                    }
                    _ => {}
                }
            }
            if !changed {
                break;
            }
        }

        let concurrent_traces = actions
            .iter()
            .map(|action| action.trace_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|trace_id| {
                self.tasks
                    .values()
                    .filter(|task| task.trace_id == *trace_id && task.active)
                    .count()
                    > 1
            })
            .collect::<BTreeSet<_>>();
        let mut ambiguous_traces = BTreeSet::new();

        // Write task IDs back to actions.
        for action in actions {
            if let Some(task_id) = task_by_action.get(&(action.trace_id, action.action_id.clone()))
            {
                action
                    .attributes
                    .insert(attr_keys::agent_turn::TASK_ID.to_owned(), task_id.clone());
            } else if action
                .attributes
                .contains_key(attr_keys::agent_turn::TASK_ID)
            {
                // Do not persist an unverifiable task ID as trusted context.
                // It could otherwise mislead later consumers even though the
                // detector rejected the update.
                action.attributes.remove(attr_keys::agent_turn::TASK_ID);
                ambiguous_traces.insert(action.trace_id);
            } else if concurrent_traces.contains(&action.trace_id) {
                ambiguous_traces.insert(action.trace_id);
            }
        }
        ambiguous_traces
    }

    /// Resolves a task from explicit action metadata or an unambiguous active task.
    fn resolve_task_id(
        &self,
        trace_id: TraceId,
        attributes: &BTreeMap<String, String>,
    ) -> Option<String> {
        // Use an action task ID only while that task accepts delayed actions.
        if let Some(task_id) = attributes.get(attr_keys::agent_turn::TASK_ID) {
            return self
                .tasks
                .get(&(trace_id, task_id.clone()))
                .is_some_and(|task| task.active || task.open_interval.is_some())
                .then(|| task_id.clone());
        }
        let mut candidates = self.tasks.values().filter(|task| {
            task.trace_id == trace_id && (task.active || task.open_interval.is_some())
        });
        // Fall back only when exactly one task accepts progress.
        let task = candidates.next()?;
        candidates.next().is_none().then(|| task.task_id.clone())
    }
}
