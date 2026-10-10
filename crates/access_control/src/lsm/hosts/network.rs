use super::super::controller::LsmController;
use plugin_system::*;

impl NetworkPolicyHost for LsmController {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError> {
        Ok(self
            .lock()
            .map_err(Self::host_error)?
            .stores
            .network
            .revision())
    }
    fn rules_list(
        &self,
        filter: NetworkPolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<NetworkPolicyListResult, PluginRuntimeError> {
        let state = self.lock().map_err(Self::host_error)?;
        state
            .stores
            .network
            .list(filter, cursor.as_deref(), limit)
            .map_err(Self::host_error)
    }
    fn rules_match_dry_run(
        &self,
        request: NetworkPolicyMatchDryRunRequest,
    ) -> Result<NetworkPolicyMatchDryRunResult, PluginRuntimeError> {
        let state = self.lock().map_err(Self::host_error)?;
        let default =
            NetworkPolicyDecision::from_wire(self.network_config.default_decision.as_str())
                .map_err(Self::host_error)?;
        state
            .stores
            .network
            .match_dry_run(request, default)
            .map_err(Self::host_error)
    }
    fn rules_validate(
        &self,
        owner: &str,
        grants: &[NetworkPolicyRulesApplyGrant],
        request: &NetworkPolicyApplyRequest,
    ) -> Result<NetworkPolicyApplyResult, PluginRuntimeError> {
        self.network_mutation(owner, grants, request.clone(), false)
    }
    fn rules_apply(
        &self,
        owner: &str,
        grants: &[NetworkPolicyRulesApplyGrant],
        request: NetworkPolicyApplyRequest,
    ) -> Result<NetworkPolicyApplyResult, PluginRuntimeError> {
        self.network_mutation(owner, grants, request, true)
    }
}

impl LsmController {
    fn network_mutation(
        &self,
        owner: &str,
        grants: &[NetworkPolicyRulesApplyGrant],
        request: NetworkPolicyApplyRequest,
        publish: bool,
    ) -> Result<NetworkPolicyApplyResult, PluginRuntimeError> {
        if !self.network_config.enabled {
            return Err(Self::host_error("LSM network control is disabled"));
        }
        let mut state = self.lock().map_err(Self::host_error)?;
        let validation =
            (!publish).then(|| state.stores.network.validate_apply(owner, grants, &request));
        let mut candidate = state.stores.clone();
        let result = candidate.network.apply(owner, grants, request);
        if result.applied_count > 0 {
            self.commit(&mut state, candidate, publish)
                .map_err(Self::host_error)?;
        }
        Ok(validation.unwrap_or(result))
    }
}
