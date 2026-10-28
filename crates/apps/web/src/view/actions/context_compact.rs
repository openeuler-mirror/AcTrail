use std::collections::{BTreeMap, BTreeSet};

use model_core::ids::TraceId;
use semantic_action::{
    SemanticAction, SemanticActionKind, SemanticActionLinkRole, attr_keys as attrs,
};
use storage_core::StorageBackend;

/// Resolves exact Compact markers for their implementation calls.
pub(in crate::view) struct ContextCompactDisplay {
    by_implementation: BTreeMap<String, CompactImplementation>,
    // Read-only lookup into the same pairs; released with this display request.
    by_compact: BTreeMap<String, String>,
}

struct CompactImplementation {
    compact: SemanticAction,
    call: SemanticAction,
}

pub(in crate::view) struct ContextCompactRequests {
    pub(in crate::view) implementations: BTreeMap<String, String>,
    pub(in crate::view) adoptions: BTreeMap<String, String>,
    pub(in crate::view) sources: BTreeMap<String, String>,
}

impl ContextCompactDisplay {
    /// Command fallback counts omit compact rows already positioned through
    /// their implementation calls.
    pub(in crate::view) fn excluded_fallback_kinds() -> [&'static str; 1] {
        [SemanticActionKind::AgentContextCompact.as_str()]
    }

    /// Navigation follows the implementation's original parents.
    pub(in crate::view) fn navigation_parent_roles<'a>(roles: &[&'a str]) -> Vec<&'a str> {
        roles
            .iter()
            .copied()
            .filter(|role| {
                *role != SemanticActionLinkRole::AgentContextCompactImplementationLlmCall.as_str()
            })
            .collect()
    }

    /// Resolve each relation once from persisted metadata and exact endpoints.
    /// Only missing records are read, in batches, for this display request.
    pub(in crate::view) fn for_actions<'a>(
        storage: &dyn StorageBackend,
        trace_id: TraceId,
        actions: impl IntoIterator<Item = &'a SemanticAction>,
    ) -> Result<Self, String> {
        let mut calls = BTreeMap::new();
        let mut compacts = BTreeMap::new();
        let mut missing = BTreeSet::new();
        for action in actions {
            match action.kind {
                SemanticActionKind::LlmCall => {
                    calls.insert(action.action_id.as_str(), action);
                }
                SemanticActionKind::AgentContextCompact => {
                    if action.attributes.contains_key(
                        attrs::agent_context_compact::IMPLEMENTATION_LLM_CALL_ACTION_ID,
                    ) {
                        compacts.insert(action.action_id.clone(), action.clone());
                    } else {
                        missing.insert(action.action_id.clone());
                    }
                }
                _ => {}
            }
        }
        for id in calls.keys() {
            let candidate_id = SemanticAction::context_compact_action_id(id);
            if !compacts.contains_key(&candidate_id) {
                missing.insert(candidate_id);
            }
        }
        let ids = missing.iter().map(String::as_str).collect::<Vec<_>>();
        for action in Self::read_actions(storage, trace_id, &ids)? {
            if action.kind == SemanticActionKind::AgentContextCompact {
                compacts.insert(action.action_id.clone(), action);
            }
        }
        let missing_calls = compacts
            .values()
            .filter_map(|compact| {
                compact
                    .attributes
                    .get(attrs::agent_context_compact::IMPLEMENTATION_LLM_CALL_ACTION_ID)
            })
            .filter(|id| !calls.contains_key(id.as_str()))
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let endpoints = Self::read_actions(
            storage,
            trace_id,
            &missing_calls.into_iter().collect::<Vec<_>>(),
        )?;
        for action in &endpoints {
            calls.insert(action.action_id.as_str(), action);
        }
        let mut by_implementation = BTreeMap::new();
        for compact in compacts.into_values() {
            let Some(id) = compact
                .attributes
                .get(attrs::agent_context_compact::IMPLEMENTATION_LLM_CALL_ACTION_ID)
            else {
                continue;
            };
            let Some(call) = calls
                .get(id.as_str())
                .copied()
                .filter(|call| compact.references_context_compact_implementation(call))
            else {
                continue;
            };
            by_implementation
                .entry(id.clone())
                .or_insert_with(|| CompactImplementation {
                    compact,
                    call: call.clone(),
                });
        }
        let by_compact = by_implementation
            .iter()
            .map(|(id, entry)| (entry.compact.action_id.clone(), id.clone()))
            .collect();
        Ok(Self {
            by_implementation,
            by_compact,
        })
    }

    pub(super) fn project_root_rows(
        &self,
        storage: &dyn StorageBackend,
        trace_id: TraceId,
        rows: &mut [super::DisplayChild],
        roles: &[&str],
    ) -> Result<(), String> {
        // Storage places a root Compact and its implementation in one slot.
        // Render that slot as the call while preserving the page's offsets.
        let calls = self
            .by_implementation
            .values()
            .map(|entry| (&entry.compact.action_id, &entry.call))
            .collect::<BTreeMap<_, _>>();
        for row in rows {
            if let Some(call) = calls.get(&row.action.action_id) {
                row.child_count = storage
                    .semantic_action_child_count(trace_id, &call.action_id, roles)
                    .map_err(|error| format!("count implementation children failed: {error:?}"))?;
                row.action = (*call).clone();
                if let Some(link) = row.link.as_mut() {
                    link.child_action_id = call.action_id.clone();
                }
            }
        }
        Ok(())
    }

    pub(in crate::view) fn for_implementation_ids<'a>(
        storage: &dyn StorageBackend,
        trace_id: TraceId,
        ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, String> {
        let ids = ids.into_iter().collect::<BTreeSet<_>>();
        let mut actions =
            Self::candidates(storage, trace_id, &ids.iter().copied().collect::<Vec<_>>())?;
        actions.retain(|action| {
            action
                .attributes
                .get(attrs::agent_context_compact::IMPLEMENTATION_LLM_CALL_ACTION_ID)
                .is_some_and(|id| ids.contains(id.as_str()))
        });
        Self::for_actions(storage, trace_id, &actions)
    }

    fn candidates(
        storage: &dyn StorageBackend,
        trace_id: TraceId,
        implementation_ids: &[&str],
    ) -> Result<Vec<SemanticAction>, String> {
        // The producer's deterministic ID locates a candidate through the
        // unique action ID index. Persisted attributes and endpoint identity
        // establish the relation; the ID spelling alone never does.
        let ids = implementation_ids
            .iter()
            .map(|id| SemanticAction::context_compact_action_id(id))
            .collect::<Vec<_>>();
        let ids = ids.iter().map(String::as_str).collect::<Vec<_>>();
        Self::read_actions(storage, trace_id, &ids)
    }

    fn read_actions(
        storage: &dyn StorageBackend,
        trace_id: TraceId,
        ids: &[&str],
    ) -> Result<Vec<SemanticAction>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        storage
            .semantic_actions_by_ids_lite(trace_id, ids)
            .map_err(|error| format!("read context compaction actions failed: {error:?}"))
    }

    pub(in crate::view) fn implementation_calls(&self) -> impl Iterator<Item = &SemanticAction> {
        self.by_implementation.values().map(|entry| &entry.call)
    }

    pub(in crate::view) fn compact_for_implementation(
        &self,
        implementation_id: &str,
    ) -> Option<&SemanticAction> {
        self.by_implementation
            .get(implementation_id)
            .map(|entry| &entry.compact)
    }

    pub(in crate::view) fn action_json(&self, action: &SemanticAction, lite: bool) -> String {
        let relation = match action.kind {
            SemanticActionKind::LlmCall => self.by_implementation.get(&action.action_id),
            SemanticActionKind::AgentContextCompact => self
                .by_compact
                .get(&action.action_id)
                .and_then(|id| self.by_implementation.get(id)),
            _ => None,
        };
        super::serialization::render_action_json(
            action,
            lite,
            relation.map(|entry| {
                (
                    entry.compact.action_id.as_str(),
                    entry.call.action_id.as_str(),
                )
            }),
        )
    }

    pub(in crate::view) fn request_compactions(
        &self,
        actions: &[SemanticAction],
    ) -> ContextCompactRequests {
        let by_id = actions
            .iter()
            .map(|action| (action.action_id.as_str(), action))
            .collect::<BTreeMap<_, _>>();
        let mut implementations = BTreeMap::new();
        let mut adoptions = BTreeMap::new();
        let mut sources = BTreeMap::new();
        for entry in self.by_implementation.values() {
            let compact = &entry.compact;
            let call = &entry.call;
            if let Some(request) = call
                .attributes
                .get(attrs::llm_call::REQUEST_ACTION_ID)
                .and_then(|id| by_id.get(id.as_str()))
                .filter(|request| {
                    request.kind == SemanticActionKind::LlmRequest
                        && request.trace_id == compact.trace_id
                        && request.process == compact.process
                })
            {
                implementations.insert(request.action_id.clone(), compact.action_id.clone());
                if let Some(source) = compact
                    .attributes
                    .get(attrs::agent_context_compact::PRE_CONTEXT_REQUEST_ACTION_ID)
                    .and_then(|id| by_id.get(id.as_str()))
                    .filter(|source| {
                        source.kind == SemanticActionKind::LlmRequest
                            && source.trace_id == compact.trace_id
                            && source.process == compact.process
                            && source.start_time < request.start_time
                    })
                {
                    sources.insert(request.action_id.clone(), source.action_id.clone());
                }
            }
            if let Some(request) = compact
                .attributes
                .get(attrs::agent_context_compact::POST_CONTEXT_REQUEST_ACTION_ID)
                .and_then(|id| by_id.get(id.as_str()))
                .filter(|request| {
                    request.kind == SemanticActionKind::LlmRequest
                        && request.trace_id == compact.trace_id
                        && request.process == compact.process
                        && call.start_time < request.start_time
                })
            {
                adoptions.insert(request.action_id.clone(), compact.action_id.clone());
            }
        }
        ContextCompactRequests {
            implementations,
            adoptions,
            sources,
        }
    }
}
