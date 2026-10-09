use super::super::controller::LsmController;
use plugin_system::*;

impl CommandPolicyHost for LsmController {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError> {
        Ok(self
            .lock()
            .map_err(Self::host_error)?
            .stores
            .command
            .revision())
    }
    fn rules_list(
        &self,
        filter: CommandPolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<CommandPolicyListResult, PluginRuntimeError> {
        let state = self.lock().map_err(Self::host_error)?;
        state
            .stores
            .command
            .list(filter, cursor.as_deref(), limit)
            .map_err(Self::host_error)
    }
    fn rules_match_dry_run(
        &self,
        request: CommandPolicyMatchDryRunRequest,
    ) -> Result<CommandPolicyMatchDryRunResult, PluginRuntimeError> {
        let state = self.lock().map_err(Self::host_error)?;
        let default =
            CommandPolicyDecision::from_wire(self.command_config.default_decision.as_str())
                .map_err(Self::host_error)?;
        state
            .stores
            .command
            .match_dry_run(request, default)
            .map_err(Self::host_error)
    }
    fn rules_validate(
        &self,
        owner: &str,
        grants: &[CommandPolicyRulesApplyGrant],
        request: &CommandPolicyApplyRequest,
    ) -> Result<CommandPolicyApplyResult, PluginRuntimeError> {
        self.command_mutation(owner, grants, request.clone(), false)
    }
    fn rules_apply(
        &self,
        owner: &str,
        grants: &[CommandPolicyRulesApplyGrant],
        request: CommandPolicyApplyRequest,
    ) -> Result<CommandPolicyApplyResult, PluginRuntimeError> {
        self.command_mutation(owner, grants, request, true)
    }
}

impl LsmController {
    fn command_mutation(
        &self,
        owner: &str,
        grants: &[CommandPolicyRulesApplyGrant],
        request: CommandPolicyApplyRequest,
        publish: bool,
    ) -> Result<CommandPolicyApplyResult, PluginRuntimeError> {
        if !self.command_config.enabled {
            return Err(Self::host_error("LSM command control is disabled"));
        }
        let mut state = self.lock().map_err(Self::host_error)?;
        let validation =
            (!publish).then(|| state.stores.command.validate_apply(owner, grants, &request));
        let mut candidate = state.stores.clone();
        let result = candidate.command.apply(owner, grants, request);
        if result.applied_count > 0 {
            self.commit(&mut state, candidate, publish)
                .map_err(Self::host_error)?;
        }
        Ok(validation.unwrap_or(result))
    }
}
