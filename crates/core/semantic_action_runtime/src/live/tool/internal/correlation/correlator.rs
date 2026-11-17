//! Agent-invocation classification of declared tool calls.

use std::collections::BTreeSet;

use config_core::daemon::AgentInvocationConfig;

pub(in crate::live::tool) struct AgentInvocationCorrelator {
    enabled: bool,
    agent_tool_names: BTreeSet<String>,
}

impl AgentInvocationCorrelator {
    pub(in crate::live::tool) fn new(config: AgentInvocationConfig) -> Self {
        Self {
            enabled: config.enabled,
            agent_tool_names: config.tool_names.into_iter().collect(),
        }
    }

    pub(in crate::live::tool) fn projects_invocation(&self, tool_name: &str) -> bool {
        self.enabled && self.agent_tool_names.contains(tool_name)
    }
}
