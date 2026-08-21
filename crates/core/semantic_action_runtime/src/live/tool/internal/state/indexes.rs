//! Bounded-cost lookup indexes for live tool interaction state.

use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ToolStateKind {
    ToolCall,
    ToolResult,
    AgentInvocation,
}

pub(super) struct ToolStateEviction {
    pub(super) kind: ToolStateKind,
    pub(super) action_id: String,
}

#[derive(Default)]
pub(super) struct ToolTraceEntries {
    pub(super) tool_calls: BTreeSet<String>,
    pub(super) tool_results: BTreeSet<String>,
    pub(super) agent_invocations: BTreeSet<String>,
}

#[derive(Default)]
pub(super) struct ToolInteractionIndexes {
    traces: BTreeMap<TraceId, ToolTraceEntries>,
    tool_call_ids: BTreeMap<(TraceId, String), BTreeSet<String>>,
    prompt_hashes: BTreeMap<(TraceId, String), BTreeSet<String>>,
    prompt_previews: BTreeMap<(TraceId, String), BTreeSet<String>>,
    order: BTreeMap<TraceId, BTreeSet<(u64, ToolStateKind, String)>>,
    positions: BTreeMap<(TraceId, ToolStateKind, String), u64>,
    next_sequence: u64,
}

impl ToolInteractionIndexes {
    pub(super) fn admit(
        &mut self,
        trace_id: TraceId,
        kind: ToolStateKind,
        action_id: &str,
        max_entries_per_trace: usize,
    ) -> Option<ToolStateEviction> {
        let position_key = (trace_id, kind, action_id.to_string());
        if self.positions.contains_key(&position_key) {
            return None;
        }
        let eviction = if self
            .order
            .get(&trace_id)
            .is_some_and(|entries| entries.len() >= max_entries_per_trace)
        {
            self.evict_oldest(trace_id)
        } else {
            None
        };
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.order
            .entry(trace_id)
            .or_default()
            .insert((sequence, kind, action_id.to_string()));
        self.positions.insert(position_key, sequence);
        eviction
    }

    pub(super) fn record_tool_call(
        &mut self,
        trace_id: TraceId,
        action_id: &str,
        previous_tool_call_id: Option<&str>,
        tool_call_id: Option<&str>,
    ) {
        self.traces
            .entry(trace_id)
            .or_default()
            .tool_calls
            .insert(action_id.to_string());
        if previous_tool_call_id != tool_call_id {
            if let Some(previous) = previous_tool_call_id {
                Self::remove_lookup(&mut self.tool_call_ids, trace_id, previous, action_id);
            }
        }
        if let Some(tool_call_id) = tool_call_id {
            self.tool_call_ids
                .entry((trace_id, tool_call_id.to_string()))
                .or_default()
                .insert(action_id.to_string());
        }
    }

    pub(super) fn tool_call_candidates(
        &self,
        trace_id: TraceId,
        tool_call_id: &str,
    ) -> impl Iterator<Item = &String> {
        self.tool_call_ids
            .get(&(trace_id, tool_call_id.to_string()))
            .into_iter()
            .flatten()
    }

    pub(super) fn record_tool_result(&mut self, trace_id: TraceId, action_id: &str) {
        self.traces
            .entry(trace_id)
            .or_default()
            .tool_results
            .insert(action_id.to_string());
    }

    pub(super) fn record_agent_invocation(
        &mut self,
        trace_id: TraceId,
        action_id: &str,
        prompt_hashes: &BTreeSet<String>,
        prompt_preview: Option<&str>,
    ) {
        self.traces
            .entry(trace_id)
            .or_default()
            .agent_invocations
            .insert(action_id.to_string());
        for hash in prompt_hashes {
            self.prompt_hashes
                .entry((trace_id, hash.clone()))
                .or_default()
                .insert(action_id.to_string());
        }
        if let Some(preview) = prompt_preview {
            self.prompt_previews
                .entry((trace_id, preview.to_string()))
                .or_default()
                .insert(action_id.to_string());
        }
    }

    pub(super) fn agent_child_candidates(
        &self,
        trace_id: TraceId,
        request_hash: Option<&str>,
        request_preview: Option<&str>,
    ) -> BTreeSet<String> {
        let mut candidates = BTreeSet::new();
        if let Some(hash) = request_hash
            && let Some(action_ids) = self.prompt_hashes.get(&(trace_id, hash.to_string()))
        {
            candidates.extend(action_ids.iter().cloned());
        }
        if let Some(preview) = request_preview
            && let Some(action_ids) = self.prompt_previews.get(&(trace_id, preview.to_string()))
        {
            candidates.extend(action_ids.iter().cloned());
        }
        candidates
    }

    pub(super) fn unlink_agent_candidate(
        &mut self,
        trace_id: TraceId,
        action_id: &str,
        prompt_hashes: &BTreeSet<String>,
        prompt_preview: Option<&str>,
    ) {
        for hash in prompt_hashes {
            Self::remove_lookup(&mut self.prompt_hashes, trace_id, hash, action_id);
        }
        if let Some(preview) = prompt_preview {
            Self::remove_lookup(&mut self.prompt_previews, trace_id, preview, action_id);
        }
    }

    pub(super) fn take_trace(&mut self, trace_id: TraceId) -> ToolTraceEntries {
        let entries = self.traces.remove(&trace_id).unwrap_or_default();
        self.order.remove(&trace_id);
        for (kind, action_ids) in [
            (ToolStateKind::ToolCall, &entries.tool_calls),
            (ToolStateKind::ToolResult, &entries.tool_results),
            (ToolStateKind::AgentInvocation, &entries.agent_invocations),
        ] {
            for action_id in action_ids {
                self.positions.remove(&(trace_id, kind, action_id.clone()));
            }
        }
        entries
    }

    pub(super) fn remove_tool_call_id(
        &mut self,
        trace_id: TraceId,
        tool_call_id: &str,
        action_id: &str,
    ) {
        Self::remove_lookup(&mut self.tool_call_ids, trace_id, tool_call_id, action_id);
    }

    fn remove_lookup(
        lookup: &mut BTreeMap<(TraceId, String), BTreeSet<String>>,
        trace_id: TraceId,
        value: &str,
        action_id: &str,
    ) {
        let key = (trace_id, value.to_string());
        let remove_entry = lookup.get_mut(&key).is_some_and(|action_ids| {
            action_ids.remove(action_id);
            action_ids.is_empty()
        });
        if remove_entry {
            lookup.remove(&key);
        }
    }

    fn evict_oldest(&mut self, trace_id: TraceId) -> Option<ToolStateEviction> {
        let (sequence, kind, action_id) = self.order.get_mut(&trace_id)?.pop_first()?;
        self.positions.remove(&(trace_id, kind, action_id.clone()));
        if self.order.get(&trace_id).is_some_and(BTreeSet::is_empty) {
            self.order.remove(&trace_id);
        }
        let trace = self.traces.get_mut(&trace_id)?;
        match kind {
            ToolStateKind::ToolCall => {
                trace.tool_calls.remove(&action_id);
            }
            ToolStateKind::ToolResult => {
                trace.tool_results.remove(&action_id);
            }
            ToolStateKind::AgentInvocation => {
                trace.agent_invocations.remove(&action_id);
            }
        }
        let _ = sequence;
        Some(ToolStateEviction { kind, action_id })
    }
}
