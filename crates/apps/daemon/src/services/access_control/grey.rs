//! Adapt LSM decisions to the active control plugin instance.
use access_control::lsm::LsmGreyHandler;
use plugin_system::{ControlDecisionBudget, ControlDecisionRequest, ControlVerdict};

use crate::services::control_runtime::WeakControlPluginRuntime;

pub(in crate::services) struct LsmPluginDecider(pub(in crate::services) WeakControlPluginRuntime);

impl LsmGreyHandler for LsmPluginDecider {
    fn decide(
        &self,
        target: &str,
        request: ControlDecisionRequest,
        budget: ControlDecisionBudget,
    ) -> Result<ControlVerdict, String> {
        let Some(plugins) = self.0.upgrade() else {
            return Ok(ControlVerdict::Deny);
        };
        let Some((index, _)) = plugins.active_instance_registration(target) else {
            return Ok(ControlVerdict::Deny);
        };
        let result = plugins.decide_by_index(index, request, budget);
        // An unloaded instance cannot authorize an in-flight request.
        if !plugins.is_instance_index_active(index) {
            return Ok(ControlVerdict::Deny);
        }
        result
            .map(|response| response.verdict)
            .map_err(|error| format!("{}: {}", error.code, error.message))
    }
}
