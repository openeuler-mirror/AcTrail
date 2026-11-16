//! Semantic decision boundary for the LSM grey runtime.
use plugin_system::{ControlDecisionBudget, ControlDecisionRequest, ControlVerdict};

pub trait LsmGreyHandler: Send + Sync {
    fn decide(
        &self,
        target: &str,
        request: ControlDecisionRequest,
        budget: ControlDecisionBudget,
    ) -> Result<ControlVerdict, String>;
}
