use super::{SemanticAction, SemanticActionKind};
use crate::attr_keys;

impl SemanticAction {
    /// Derives a compact record's ID from its implementation call ID.
    pub fn context_compact_action_id(llm_call_action_id: &str) -> String {
        let prefix = llm_call_action_id
            .strip_suffix(":llm.call")
            .unwrap_or(llm_call_action_id);
        format!("{prefix}:agent.context_compaction")
    }

    /// Whether this compact record identifies the exact implementation call.
    /// Completion requirements belong to the producer admitting the relation.
    pub fn references_context_compact_implementation(&self, implementation: &Self) -> bool {
        self.kind == SemanticActionKind::AgentContextCompact
            && implementation.kind == SemanticActionKind::LlmCall
            && self.trace_id == implementation.trace_id
            && self.process == implementation.process
            && self
                .attributes
                .get(attr_keys::agent_context_compact::IMPLEMENTATION_LLM_CALL_ACTION_ID)
                == Some(&implementation.action_id)
    }
}
