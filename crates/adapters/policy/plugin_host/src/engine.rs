//! Policy evaluation entry boundary for runtime consumers.

use policy_evaluate_contract::decision::PolicyDecision;
use policy_evaluate_contract::evaluate::{PolicyEvaluator, PolicyInput};

use crate::decisions::merge_decisions;
use crate::registry::PluginRegistry;

pub struct PluginPolicyEngine {
    registry: PluginRegistry,
}

impl PluginPolicyEngine {
    pub fn new(registry: PluginRegistry) -> Self {
        Self { registry }
    }
}

impl PolicyEvaluator for PluginPolicyEngine {
    fn evaluate(&self, input: &PolicyInput) -> PolicyDecision {
        if self.registry.is_empty() {
            return PolicyDecision::allow();
        }

        merge_decisions(
            self.registry
                .plugins()
                .iter()
                .map(|plugin| plugin.evaluate(input)),
        )
    }
}
