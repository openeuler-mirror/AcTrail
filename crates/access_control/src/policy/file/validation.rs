use super::model::StoredPolicyRule;
use super::scope::PathScope;
use plugin_system::{
    FilePolicyApplyError, FilePolicyApplyResult, FilePolicyApplyStatus, FilePolicyDecision,
    FilePolicyRuleDraft, FilePolicyRulesApplyGrant,
};

pub(super) struct FilePolicyValidation;
impl FilePolicyValidation {
    pub(super) fn validate_bound_grant(
        grants: &[FilePolicyRulesApplyGrant],
        rule: &StoredPolicyRule,
    ) -> Result<(), String> {
        for grant in grants
            .iter()
            .filter(|grant| grant.decision == rule.decision)
        {
            let scope = PathScope::normalized_scope(&grant.path_scope)?;
            if scope.contains_scope(&rule.scope)
                && PathScope::absolute_scope(&grant.path_scope)
                    .and_then(|scope| scope.bind())
                    .is_ok_and(|scope| scope.contains_scope(&rule.execution_scope))
            {
                return Ok(());
            }
        }
        Err(format!(
            "missing file-policy.rules.apply grant for bound target {}",
            rule.execution_scope.display_path()
        ))
    }

    pub(super) fn validate_draft_grant(
        grants: &[FilePolicyRulesApplyGrant],
        draft: &FilePolicyRuleDraft,
    ) -> Result<(), String> {
        let scope = PathScope::normalized_scope(&draft.path)?;
        Self::validate_rule_grant(grants, draft.decision, &scope)
    }

    pub(super) fn validate_rule_grant(
        grants: &[FilePolicyRulesApplyGrant],
        decision: FilePolicyDecision,
        scope: &PathScope,
    ) -> Result<(), String> {
        if grants.iter().any(|grant| {
            grant.decision == decision
                && PathScope::normalized_scope(&grant.path_scope)
                    .map(|grant_scope| grant_scope.contains_scope(scope))
                    .unwrap_or(false)
        }) {
            return Ok(());
        }
        Err(format!(
            "missing file-policy.rules.apply grant for {} {}",
            decision.as_str(),
            scope.display_path()
        ))
    }

    pub(super) fn apply_result_from_errors(
        revision: u64,
        applied_count: u32,
        errors: Vec<FilePolicyApplyError>,
    ) -> FilePolicyApplyResult {
        FilePolicyApplyResult {
            status: if errors.is_empty() {
                FilePolicyApplyStatus::Accepted
            } else {
                FilePolicyApplyStatus::Rejected
            },
            new_revision: revision,
            applied_count,
            rejected_count: errors.len() as u32,
            errors,
        }
    }
}
