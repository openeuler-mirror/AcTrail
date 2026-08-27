//! Atomic state ownership for live tool interaction correlation.

use std::collections::{BTreeMap, BTreeSet};
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
    pub(in crate::live::tool) eviction: Option<StateEviction>,
}

pub(in crate::live::tool) struct ToolCallCandidate {
    pub(in crate::live::tool) action_id: String,
}

pub(in crate::live::tool) struct FinalizedInvocation {
    pub(in crate::live::tool) action: SemanticAction,
    pub(in crate::live::tool) tool_call: Option<SemanticAction>,
}

pub(in crate::live::tool) struct ToolInteractionState {
    tool_calls: BTreeMap<(TraceId, String), ToolCallRecord>,
    tool_results: BTreeMap<(TraceId, String), SemanticAction>,
    agent_invocations: BTreeMap<(TraceId, String), AgentInvocationRecord>,
    indexes: ToolInteractionIndexes,
    max_entries_per_trace: usize,
}

impl ToolInteractionState {
    pub(in crate::live::tool) fn new(max_entries_per_trace: u32) -> Self {
        Self {
            tool_calls: BTreeMap::new(),
            tool_results: BTreeMap::new(),
            agent_invocations: BTreeMap::new(),
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
        StateMutation { changed, eviction }
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
        let changed = self.tool_results.get(&key) != Some(&action);
        self.tool_results.insert(key, action);
        self.indexes.record_tool_result(trace_id, &action_id);
        StateMutation { changed, eviction }
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
        prompt_message_hashes: BTreeSet<String>,
        prompt_preview: Option<String>,
    ) -> StateMutation {
        let trace_id = action.trace_id;
        let key = (trace_id, tool_call_action_id.to_string());
        if self.agent_invocations.contains_key(&key) {
            return StateMutation {
                changed: false,
                eviction: None,
            };
        }
        let eviction = self.admit(
            trace_id,
            ToolStateKind::AgentInvocation,
            tool_call_action_id,
            action.start_time,
        );
        self.indexes.record_agent_invocation(
            trace_id,
            tool_call_action_id,
            &prompt_message_hashes,
            prompt_preview.as_deref(),
        );
        self.agent_invocations.insert(
            key,
            AgentInvocationRecord {
                action: action.clone(),
                prompt_message_hashes,
                prompt_preview,
                child_linked: false,
            },
        );
        StateMutation {
            changed: true,
            eviction,
        }
    }

    pub(in crate::live::tool) fn tool_call_candidates(
        &self,
        trace_id: TraceId,
        tool_call_id: &str,
        process: &ProcessIdentity,
    ) -> Vec<ToolCallCandidate> {
        self.indexes
            .tool_call_candidates(trace_id, tool_call_id)
            .filter_map(|action_id| {
                self.tool_calls
                    .get(&(trace_id, action_id.clone()))
                    .filter(|record| record.action.process == *process)
                    .map(|_| ToolCallCandidate {
                        action_id: action_id.clone(),
                    })
            })
            .collect()
    }

    pub(in crate::live::tool) fn tool_call_action(
        &self,
        trace_id: TraceId,
        action_id: &str,
    ) -> Option<SemanticAction> {
        self.tool_calls
            .get(&(trace_id, action_id.to_string()))
            .map(|record| record.action.clone())
    }

    pub(in crate::live::tool) fn complete_agent_invocation(
        &mut self,
        trace_id: TraceId,
        tool_call_action_id: &str,
        is_error: bool,
        request: &SemanticAction,
    ) -> Option<SemanticAction> {
        let invocation = self
            .agent_invocations
            .get_mut(&(trace_id, tool_call_action_id.to_string()))?;
        if invocation.action.status != SemanticActionStatus::InProgress {
            return None;
        }
        invocation.action.status = if is_error {
            SemanticActionStatus::Error
        } else {
            SemanticActionStatus::Success
        };
        invocation.action.completeness = request.completeness;
        invocation.action.end_time = request.end_time.or(Some(request.start_time));
        Some(invocation.action.clone())
    }

    pub(in crate::live::tool) fn agent_child_candidates(
        &self,
        trace_id: TraceId,
        request_hash: Option<&str>,
        request_preview: Option<&str>,
    ) -> Vec<String> {
        self.indexes
            .agent_child_candidates(trace_id, request_hash, request_preview)
            .into_iter()
            .filter(|action_id| {
                self.agent_invocations
                    .get(&(trace_id, action_id.clone()))
                    .is_some_and(|invocation| !invocation.child_linked)
            })
            .collect()
    }

    pub(in crate::live::tool) fn link_agent_child(
        &mut self,
        trace_id: TraceId,
        tool_call_action_id: &str,
    ) -> Option<SemanticAction> {
        let key = (trace_id, tool_call_action_id.to_string());
        let invocation = self.agent_invocations.get_mut(&key)?;
        invocation.child_linked = true;
        let prompt_message_hashes = invocation.prompt_message_hashes.clone();
        let prompt_preview = invocation.prompt_preview.clone();
        let action = invocation.action.clone();
        self.indexes.unlink_agent_candidate(
            trace_id,
            tool_call_action_id,
            &prompt_message_hashes,
            prompt_preview.as_deref(),
        );
        Some(action)
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
                self.indexes.unlink_agent_candidate(
                    trace_id,
                    &eviction.action_id,
                    &invocation.prompt_message_hashes,
                    invocation.prompt_preview.as_deref(),
                );
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
            if let Some(invocation) = self
                .agent_invocations
                .remove(&(trace_id, action_id.clone()))
            {
                self.indexes.unlink_agent_candidate(
                    trace_id,
                    &action_id,
                    &invocation.prompt_message_hashes,
                    invocation.prompt_preview.as_deref(),
                );
            }
        }
    }
}
