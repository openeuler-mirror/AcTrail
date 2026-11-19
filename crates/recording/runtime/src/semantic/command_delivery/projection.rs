use std::collections::HashMap;

use export_core::{ExportDroppedRecord, ExportPublishReport};
use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionChange, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionLink, SemanticActionStatus, SemanticActionUpdate,
};

use super::{CommandDeliveryLimits, pending::PendingCommand, sizing::AccountedBytes};

pub(crate) struct CommandDelivery {
    limits: CommandDeliveryLimits,
    by_trace: HashMap<TraceId, TraceCommands>,
    count: usize,
    bytes: usize,
}

#[derive(Default)]
struct TraceCommands {
    by_id: HashMap<String, PendingCommand>,
    bytes: usize,
}

#[derive(Default)]
pub(crate) struct CompletedCommands {
    pub(crate) actions: Vec<SemanticAction>,
    pub(crate) links: Vec<SemanticActionLink>,
    drops: [u64; 3],
}

impl CommandDelivery {
    pub(crate) fn new(limits: CommandDeliveryLimits) -> Self {
        Self {
            limits,
            by_trace: HashMap::new(),
            count: 0,
            bytes: 0,
        }
    }

    pub(crate) fn complete<'a>(
        &mut self,
        trace_id: TraceId,
        actions: impl Iterator<Item = &'a SemanticAction>,
        links: &[SemanticActionLink],
        updates: &[SemanticActionUpdate],
    ) -> CompletedCommands {
        let mut result = CompletedCommands::default();
        let mut openings = actions
            .filter(|action| {
                action.kind == SemanticActionKind::CommandInvocation
                    && action.completeness == SemanticActionCompleteness::Partial
            })
            .peekable();
        if !self.by_trace.contains_key(&trace_id) && openings.peek().is_none() {
            return result;
        }
        let mut pending = self.by_trace.remove(&trace_id).unwrap_or_default();
        for action in openings {
            if pending.by_id.contains_key(&action.action_id) {
                continue;
            }
            let bytes = AccountedBytes::action(action);
            let rejection = if pending.by_id.len() >= self.limits.per_trace {
                Some(0)
            } else if self.count >= self.limits.total {
                Some(1)
            } else if bytes > self.limits.bytes.saturating_sub(self.bytes) {
                Some(2)
            } else {
                None
            };
            if let Some(reason) = rejection {
                result.drops[reason] += 1;
                continue;
            }
            pending
                .by_id
                .insert(action.action_id.clone(), PendingCommand::new(action, bytes));
            pending.bytes += bytes;
            self.bytes += bytes;
            self.count += 1;
        }
        for link in links {
            let Some(command) = pending.by_id.get(&link.child_action_id) else {
                continue;
            };
            let bytes = command.link_bytes_after(link);
            if !self.resize(&mut pending, &link.child_action_id, bytes) {
                result.drops[2] += 1;
                continue;
            }
            pending
                .by_id
                .get_mut(&link.child_action_id)
                .unwrap()
                .observe_link(link, bytes);
        }
        // Metadata can follow Lifecycle within a batch; consume it first.
        for update in updates
            .iter()
            .filter(|update| update.kind == SemanticActionKind::CommandInvocation)
        {
            let Some(bytes) = pending
                .by_id
                .get(&update.action_id)
                .and_then(|command| command.metadata_bytes_after(update))
            else {
                continue;
            };
            if !self.resize(&mut pending, &update.action_id, bytes) {
                result.drops[2] += 1;
                continue;
            }
            pending
                .by_id
                .get_mut(&update.action_id)
                .unwrap()
                .observe_metadata(update, bytes);
        }
        for update in updates {
            if update.kind != SemanticActionKind::CommandInvocation
                || !matches!(update.change,
                    SemanticActionChange::Lifecycle { completeness, status, .. }
                        if completeness != SemanticActionCompleteness::Partial && status != SemanticActionStatus::InProgress
                )
            {
                continue;
            }
            if let Some(mut command) = self.remove(&mut pending, &update.action_id) {
                command.finish(update);
                result.actions.push(command.action);
                result.links.extend(command.incoming_links);
            }
        }
        if !pending.by_id.is_empty() {
            self.by_trace.insert(trace_id, pending);
        }
        result
    }

    fn resize(&mut self, pending: &mut TraceCommands, id: &str, bytes: usize) -> bool {
        let previous = pending.by_id[id].bytes;
        let total = self.bytes.saturating_sub(previous);
        if bytes > self.limits.bytes.saturating_sub(total) {
            self.remove(pending, id);
            return false;
        }
        self.bytes = total + bytes;
        pending.bytes = pending.bytes - previous + bytes;
        true
    }

    fn remove(&mut self, pending: &mut TraceCommands, id: &str) -> Option<PendingCommand> {
        let command = pending.by_id.remove(id)?;
        pending.bytes -= command.bytes;
        self.bytes -= command.bytes;
        self.count -= 1;
        Some(command)
    }

    pub(crate) fn forget_trace(&mut self, trace_id: TraceId) {
        if let Some(pending) = self.by_trace.remove(&trace_id) {
            self.count -= pending.by_id.len();
            self.bytes -= pending.bytes;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.by_trace.clear();
        self.count = 0;
        self.bytes = 0;
    }
}

impl CompletedCommands {
    pub(crate) fn take_drop_report(&mut self, trace_id: TraceId) -> ExportPublishReport {
        let reasons = [
            "command_delivery_per_trace_capacity",
            "command_delivery_global_capacity",
            "command_delivery_byte_capacity",
        ];
        let dropped_records = std::mem::take(&mut self.drops)
            .into_iter()
            .zip(reasons)
            .filter(|(count, _)| *count != 0)
            .map(|(dropped_records, reason)| ExportDroppedRecord {
                trace_id: Some(trace_id),
                exporter: "command_delivery".into(),
                reason: reason.into(),
                queue_capacity: None,
                dropped_records,
            })
            .collect();
        ExportPublishReport {
            dropped_records,
            runtime_failures: Vec::new(),
        }
    }
}
