use super::super::model::StoredPolicyRule;
use super::super::scope::PathScope;
use super::super::types::{BuiltinIgnoreMark, EnforcementRule};
use super::EnforcementRules;
use plugin_system::{
    FilePolicyDecision, FilePolicyListFilter, FilePolicyListResult, FilePolicyMatchDryRunRequest,
    FilePolicyMatchDryRunResult, FilePolicyOperation, FilePolicyRuleView,
};
use std::path::{Path, PathBuf};

impl EnforcementRules {
    pub fn find_path(
        &self,
        operation: FilePolicyOperation,
        path: &Path,
    ) -> Option<&EnforcementRule> {
        self.effective_rules
            .iter()
            .find(|rule| rule.matches_path(operation, path))
            .map(|rule| &rule.rule)
    }

    pub fn has_operation_rules(&self, operation: FilePolicyOperation) -> bool {
        self.effective_rules
            .iter()
            .any(|rule| rule.operation.matches(operation))
    }

    pub fn find_builtin_allow(&self, path: &Path) -> Option<&EnforcementRule> {
        self.effective_rules
            .iter()
            .find(|rule| {
                rule.is_builtin_allow() && rule.matches_path(FilePolicyOperation::Open, path)
            })
            .map(|rule| &rule.rule)
    }

    pub fn install_builtin_allow_rules<I>(&mut self, rules: I) -> Result<(), String>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        for (rule_id, path) in rules {
            self.insert_builtin_allow_rule(rule_id, path)?;
        }
        self.rebuild_effective()
    }

    pub fn mark_directories(&self) -> impl Iterator<Item = &PathBuf> {
        self.mark_directories.iter()
    }

    pub fn builtin_ignore_marks(&self) -> Vec<BuiltinIgnoreMark> {
        self.effective_rules
            .iter()
            .filter(|rule| rule.is_builtin_allow())
            .map(|rule| BuiltinIgnoreMark {
                path: rule.execution_scope.display_base(),
                recursive: rule.execution_scope.is_recursive(),
            })
            .collect()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns bound execution paths in first-match order, including builtin priority.
    /// Consumers must preserve this order; the views do not expose the internal tier.
    pub fn effective_rule_views(&self) -> Vec<FilePolicyRuleView> {
        self.effective_rules
            .iter()
            .map(StoredPolicyRule::execution_view)
            .collect()
    }

    pub fn list(
        &self,
        filter: FilePolicyListFilter,
        cursor: Option<&str>,
        limit: u32,
    ) -> FilePolicyListResult {
        let start = cursor
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        let limit = usize::try_from(limit).unwrap_or(usize::MAX).max(1);
        let mut views = self
            .by_owner
            .values()
            .flat_map(|rules| rules.values())
            .filter(|rule| rule.matches_filter(&filter))
            .map(StoredPolicyRule::view)
            .collect::<Vec<_>>();
        views.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
        let total = views.len();
        let rules = views
            .into_iter()
            .skip(start)
            .take(limit)
            .collect::<Vec<_>>();
        let next = start.saturating_add(rules.len());
        FilePolicyListResult {
            rules,
            next_cursor: (next < total).then(|| next.to_string()),
            source_revision: self.revision,
        }
    }

    pub fn match_dry_run(
        &self,
        request: FilePolicyMatchDryRunRequest,
        default_decision: FilePolicyDecision,
    ) -> Result<FilePolicyMatchDryRunResult, String> {
        let path = PathScope::absolute_scope(&request.path)?.bind()?;
        let canonical_path = path.display_path();
        let matched = self.effective_rules.iter().find(|rule| {
            rule.operation.matches(request.operation) && rule.execution_scope.contains_scope(&path)
        });
        Ok(FilePolicyMatchDryRunResult {
            matched: matched.is_some(),
            decision: matched
                .map(|rule| rule.decision)
                .unwrap_or(default_decision),
            rule_id: matched.map(|rule| rule.rule_id.clone()),
            operation: request.operation,
            canonical_path,
            source_revision: self.revision,
        })
    }
}
