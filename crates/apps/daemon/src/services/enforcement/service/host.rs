use super::{FileBackend, FilePolicyServiceHost};
use config_core::daemon::EnforcementDecision;
use plugin_system::{
    FilePolicyApplyRequest, FilePolicyApplyResult, FilePolicyHost, FilePolicyListFilter,
    FilePolicyListResult, FilePolicyMatchDryRunRequest, FilePolicyMatchDryRunResult,
    FilePolicyRulesApplyGrant, PluginRuntimeError,
};

impl FilePolicyHost for FilePolicyServiceHost {
    fn rules_version_get(&self) -> Result<u64, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        Ok(backend.rules.revision())
    }

    fn rules_list(
        &self,
        filter: FilePolicyListFilter,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<FilePolicyListResult, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        Ok(backend.rules.list(filter, cursor.as_deref(), limit))
    }

    fn rules_match_dry_run(
        &self,
        request: FilePolicyMatchDryRunRequest,
    ) -> Result<FilePolicyMatchDryRunResult, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        backend
            .rules
            .match_dry_run(
                request,
                default_file_policy_decision(backend.default_decision),
            )
            .map_err(file_policy_error)
    }

    fn rules_validate(
        &self,
        owner_instance_id: &str,
        grants: &[FilePolicyRulesApplyGrant],
        request: &FilePolicyApplyRequest,
    ) -> Result<FilePolicyApplyResult, PluginRuntimeError> {
        let backend = self.lock_backend()?;
        let result = backend
            .rules
            .validate_apply(owner_instance_id, grants, request);
        if let Some(lsm) = &backend.lsm {
            let mut candidate = backend.rules.clone();
            candidate
                .apply(owner_instance_id, grants, request.clone())
                .map_err(file_policy_error)?;
            lsm.validate_file_rules(&candidate)
                .map_err(file_policy_error)?;
        }
        Ok(result)
    }

    fn rules_apply(
        &self,
        owner_instance_id: &str,
        grants: &[FilePolicyRulesApplyGrant],
        request: FilePolicyApplyRequest,
    ) -> Result<FilePolicyApplyResult, PluginRuntimeError> {
        let mut backend = self.lock_backend()?;
        let mut candidate = backend.rules.clone();
        let (result, _) = candidate
            .apply(owner_instance_id, grants, request)
            .map_err(file_policy_error)?;
        backend
            .replace_rules(candidate)
            .map_err(file_policy_error)?;
        Ok(result)
    }
}

impl FilePolicyServiceHost {
    fn lock_backend(&self) -> Result<std::sync::MutexGuard<'_, FileBackend>, PluginRuntimeError> {
        let Some(backend) = &self.backend else {
            return Err(PluginRuntimeError::new(
                "file_policy",
                "file enforcement is disabled",
            ));
        };
        backend.lock().map_err(|error| {
            PluginRuntimeError::new("file_policy", format!("lock backend: {error}"))
        })
    }
}

fn default_file_policy_decision(
    decision: EnforcementDecision,
) -> plugin_system::FilePolicyDecision {
    match decision {
        EnforcementDecision::Allow => plugin_system::FilePolicyDecision::Allow,
        EnforcementDecision::Deny => plugin_system::FilePolicyDecision::Deny,
    }
}

fn file_policy_error(message: String) -> PluginRuntimeError {
    PluginRuntimeError::new("file_policy", message)
}
