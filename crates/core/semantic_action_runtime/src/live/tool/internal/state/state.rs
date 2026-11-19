//! Atomic state ownership for live tool interaction correlation.

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionStatus, attr_keys as attrs,
};

use super::indexes::{ToolInteractionIndexes, ToolStateEviction, ToolStateKind, ToolTraceEntries};
use super::records::{AgentInvocationRecord, ToolCallRecord};

pub(in crate::live::tool) struct StateEviction {
    pub(in crate::live::tool) materialized_invocation: Option<SemanticAction>,
}

pub(in crate::live::tool) struct StateMutation {
    pub(in crate::live::tool) changed: bool,
    pub(in crate::live::tool) existing: bool,
    pub(in crate::live::tool) eviction: Option<StateEviction>,
}

/// What a tool result says about the outcome of the call it answers.
#[derive(Clone, Copy)]
pub(in crate::live::tool) struct ToolResultOutcome {
    pub(in crate::live::tool) is_error: bool,
    pub(in crate::live::tool) completeness: SemanticActionCompleteness,
    pub(in crate::live::tool) observed_at: SystemTime,
}

impl ToolResultOutcome {
    pub(in crate::live::tool) fn new(request: &SemanticAction, is_error: bool) -> Self {
        Self {
            is_error,
            completeness: request.completeness,
            observed_at: request.end_time.unwrap_or(request.start_time),
        }
    }
}

pub(in crate::live::tool) struct FinalizedInvocation {
    pub(in crate::live::tool) action: SemanticAction,
    pub(in crate::live::tool) tool_call: Option<SemanticAction>,
}

pub(in crate::live::tool) struct ToolInteractionState {
    tool_calls: BTreeMap<(TraceId, String), ToolCallRecord>,
    tool_results: BTreeMap<(TraceId, String), SemanticAction>,
    agent_invocations: BTreeMap<(TraceId, String), AgentInvocationRecord>,
    /// Outcomes of results processed before the response that declares their
    /// call, keyed by tool call id. Requests and responses on different
    /// connections are not delivered in capture order.
    early_results: BTreeMap<(TraceId, String), (ProcessIdentity, ToolResultOutcome)>,
    indexes: ToolInteractionIndexes,
    max_entries_per_trace: usize,
}

impl ToolInteractionState {
    pub(in crate::live::tool) fn new(max_entries_per_trace: u32) -> Self {
        Self {
            tool_calls: BTreeMap::new(),
            tool_results: BTreeMap::new(),
            agent_invocations: BTreeMap::new(),
            early_results: BTreeMap::new(),
            indexes: ToolInteractionIndexes::default(),
            max_entries_per_trace: usize::try_from(max_entries_per_trace)
                .expect("validated LLM tool state limit must fit usize"),
        }
    }

    pub(in crate::live::tool) fn record_tool_call(
        &mut self,
        action: SemanticAction,
        tool_call_id: Option<String>,
    ) -> StateMutation {
        let trace_id = action.trace_id;
        let action_id = action.action_id.clone();
        let eviction = self.admit(
            trace_id,
            ToolStateKind::ToolCall,
            &action_id,
            action.start_time,
        );
        let key = (trace_id, action_id.clone());
        let existing = self.tool_calls.contains_key(&key);
        let changed = self
            .tool_calls
            .get(&key)
            .is_none_or(|record| record.action != action);
        let previous_id = self
            .tool_calls
            .get(&key)
            .and_then(|record| record.tool_call_id.clone());
        self.tool_calls.insert(
            key,
            ToolCallRecord {
                action,
                tool_call_id: tool_call_id.clone(),
            },
        );
        self.indexes.record_tool_call(
            trace_id,
            &action_id,
            previous_id.as_deref(),
            tool_call_id.as_deref(),
        );
        StateMutation {
            changed,
            existing,
            eviction,
        }
    }

    pub(in crate::live::tool) fn record_tool_result(
        &mut self,
        action: SemanticAction,
    ) -> StateMutation {
        let trace_id = action.trace_id;
        let action_id = action.action_id.clone();
        let eviction = self.admit(
            trace_id,
            ToolStateKind::ToolResult,
            &action_id,
            action.start_time,
        );
        let key = (trace_id, action_id.clone());
        let existing = self.tool_results.contains_key(&key);
        let changed = self.tool_results.get(&key) != Some(&action);
        self.tool_results.insert(key, action);
        self.indexes.record_tool_result(trace_id, &action_id);
        StateMutation {
            changed,
            existing,
            eviction,
        }
    }

    pub(in crate::live::tool) fn has_agent_invocation(
        &self,
        trace_id: TraceId,
        tool_call_action_id: &str,
    ) -> bool {
        self.agent_invocations
            .contains_key(&(trace_id, tool_call_action_id.to_string()))
    }

    pub(in crate::live::tool) fn record_agent_invocation(
        &mut self,
        tool_call_action_id: &str,
        action: &SemanticAction,
    ) -> StateMutation {
        let trace_id = action.trace_id;
        let key = (trace_id, tool_call_action_id.to_string());
        if self.agent_invocations.contains_key(&key) {
            return StateMutation {
                changed: false,
                existing: true,
                eviction: None,
            };
        }
        let eviction = self.admit(
            trace_id,
            ToolStateKind::AgentInvocation,
            tool_call_action_id,
            action.start_time,
        );
        self.indexes
            .record_agent_invocation(trace_id, tool_call_action_id);
        self.agent_invocations.insert(
            key,
            AgentInvocationRecord {
                action: action.clone(),
            },
        );
        StateMutation {
            changed: true,
            existing: false,
            eviction,
        }
    }

    /// The tool calls a process declared under one id, oldest first.
    pub(in crate::live::tool) fn observed_tool_calls(
        &self,
        trace_id: TraceId,
        tool_call_id: &str,
        process: &ProcessIdentity,
    ) -> Vec<SemanticAction> {
        let mut calls = self
            .indexes
            .tool_call_candidates(trace_id, tool_call_id)
            .filter_map(|action_id| self.tool_calls.get(&(trace_id, action_id.clone())))
            .filter(|record| record.action.process == *process)
            .map(|record| record.action.clone())
            .collect::<Vec<_>>();
        calls.sort_by_key(|action| action.start_time);
        calls
    }

    /// Keeps the outcome of a result whose call has not been declared yet.
    pub(in crate::live::tool) fn remember_early_result(
        &mut self,
        trace_id: TraceId,
        tool_call_id: &str,
        process: ProcessIdentity,
        outcome: ToolResultOutcome,
    ) {
        let key = (trace_id, tool_call_id.to_string());
        let trace_entries = self
            .early_results
            .keys()
            .filter(|(entry_trace, _)| *entry_trace == trace_id)
            .count();
        if trace_entries < self.max_entries_per_trace || self.early_results.contains_key(&key) {
            self.early_results.insert(key, (process, outcome));
        }
    }

    pub(in crate::live::tool) fn take_early_result(
        &mut self,
        trace_id: TraceId,
        tool_call_id: &str,
        process: &ProcessIdentity,
    ) -> Option<ToolResultOutcome> {
        let key = (trace_id, tool_call_id.to_string());
        self.early_results
            .get(&key)
            .is_some_and(|(owner, _)| owner == process)
            .then(|| self.early_results.remove(&key))
            .flatten()
            .map(|(_, outcome)| outcome)
    }

    pub(in crate::live::tool) fn complete_agent_invocation(
        &mut self,
        trace_id: TraceId,
        tool_call_action_id: &str,
        outcome: ToolResultOutcome,
    ) -> Option<SemanticAction> {
        let invocation = self
            .agent_invocations
            .get_mut(&(trace_id, tool_call_action_id.to_string()))?;
        if invocation.action.status != SemanticActionStatus::InProgress {
            return None;
        }
        invocation.action.status = if outcome.is_error {
            SemanticActionStatus::Error
        } else {
            SemanticActionStatus::Success
        };
        invocation.action.completeness = outcome.completeness;
        invocation.action.end_time = Some(outcome.observed_at);
        Some(invocation.action.clone())
    }

    pub(in crate::live::tool) fn finish_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> Vec<FinalizedInvocation> {
        let trace_entries = self.indexes.take_trace(trace_id);
        let invocation_action_ids = trace_entries
            .agent_invocations
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut finalized = Vec::new();
        for tool_call_action_id in invocation_action_ids {
            let key = (trace_id, tool_call_action_id.clone());
            if let Some(invocation) = self.agent_invocations.get_mut(&key)
                && invocation.action.status == SemanticActionStatus::InProgress
            {
                invocation.action.status = SemanticActionStatus::Unknown;
                invocation.action.completeness = SemanticActionCompleteness::Partial;
                invocation.action.end_time = Some(finished_at);
                invocation.action.attributes.insert(
                    attrs::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE.to_string(),
                    "true".to_string(),
                );
                finalized.push(FinalizedInvocation {
                    action: invocation.action.clone(),
                    tool_call: self
                        .tool_calls
                        .get(&(trace_id, tool_call_action_id))
                        .map(|record| record.action.clone()),
                });
            }
        }
        self.forget_trace_records(trace_id, trace_entries);
        finalized
    }

    pub(in crate::live::tool) fn forget_trace(&mut self, trace_id: TraceId) {
        let trace_entries = self.indexes.take_trace(trace_id);
        self.forget_trace_records(trace_id, trace_entries);
    }

    fn admit(
        &mut self,
        trace_id: TraceId,
        kind: ToolStateKind,
        action_id: &str,
        observed_at: SystemTime,
    ) -> Option<StateEviction> {
        let eviction = self
            .indexes
            .admit(trace_id, kind, action_id, self.max_entries_per_trace)?;
        Some(self.apply_eviction(trace_id, eviction, observed_at))
    }

    fn apply_eviction(
        &mut self,
        trace_id: TraceId,
        eviction: ToolStateEviction,
        observed_at: SystemTime,
    ) -> StateEviction {
        let materialized_invocation = match eviction.kind {
            ToolStateKind::ToolCall => {
                if let Some(record) = self
                    .tool_calls
                    .remove(&(trace_id, eviction.action_id.clone()))
                    && let Some(tool_call_id) = record.tool_call_id
                {
                    self.indexes
                        .remove_tool_call_id(trace_id, &tool_call_id, &eviction.action_id);
                }
                None
            }
            ToolStateKind::ToolResult => {
                self.tool_results.remove(&(trace_id, eviction.action_id));
                None
            }
            ToolStateKind::AgentInvocation => {
                let Some(mut invocation) = self
                    .agent_invocations
                    .remove(&(trace_id, eviction.action_id.clone()))
                else {
                    return StateEviction {
                        materialized_invocation: None,
                    };
                };
                if invocation.action.status == SemanticActionStatus::InProgress {
                    invocation.action.status = SemanticActionStatus::Unknown;
                    invocation.action.completeness = SemanticActionCompleteness::Partial;
                    invocation.action.end_time = Some(observed_at);
                    Some(invocation.action)
                } else {
                    None
                }
            }
        };
        StateEviction {
            materialized_invocation,
        }
    }

    fn forget_trace_records(&mut self, trace_id: TraceId, entries: ToolTraceEntries) {
        self.early_results
            .retain(|(entry_trace, _), _| *entry_trace != trace_id);
        for action_id in entries.tool_calls {
            if let Some(record) = self.tool_calls.remove(&(trace_id, action_id.clone()))
                && let Some(tool_call_id) = record.tool_call_id
            {
                self.indexes
                    .remove_tool_call_id(trace_id, &tool_call_id, &action_id);
            }
        }
        for action_id in entries.tool_results {
            self.tool_results.remove(&(trace_id, action_id));
        }
        for action_id in entries.agent_invocations {
            self.agent_invocations.remove(&(trace_id, action_id));
        }
    }
}
