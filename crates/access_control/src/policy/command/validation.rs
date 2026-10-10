use super::path::CommandExecutablePath;
use super::rule::{STATIC_POLICY_OWNER, StoredCommandRule};
use crate::policy::path::PolicyPath;
use plugin_system::{
    CommandPolicyApplyError, CommandPolicyApplyRequest, CommandPolicyApplyResult,
    CommandPolicyApplyStatus, CommandPolicyDecision, CommandPolicyRuleDraft,
    CommandPolicyRulesApplyGrant,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(super) enum CommandGrantScope {
    Exact(PathBuf),
    Recursive(PathBuf),
}

pub(super) struct CommandRuleDraftValidator;

impl CommandRuleDraftValidator {
    pub(super) fn validate_id(rule_id: &str) -> Result<(), String> {
        if rule_id.trim().is_empty() {
            return Err("command rule id must not be empty".to_string());
        }
        if rule_id.chars().any(char::is_whitespace) {
            return Err(format!(
                "command rule id {rule_id} must not contain whitespace"
            ));
        }
        Ok(())
    }

    pub(super) fn validate_shape(draft: &CommandPolicyRuleDraft) -> Result<(), String> {
        match (draft.decision, draft.gray_target.as_deref()) {
            (CommandPolicyDecision::Default, _) => {
                Err("command rule decision cannot be default".to_string())
            }
            (CommandPolicyDecision::Gray, None) => {
                Err("gray command rule requires gray_target".to_string())
            }
            (CommandPolicyDecision::Gray, Some(target)) if target.trim().is_empty() => {
                Err("gray command rule target must not be empty".to_string())
            }
            (CommandPolicyDecision::Allow | CommandPolicyDecision::Deny, Some(_)) => {
                Err("allow/deny command rule must not include gray_target".to_string())
            }
            _ => Ok(()),
        }
    }
}

impl CommandGrantScope {
    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        let recursive_base = raw.strip_suffix("/**");
        let check = recursive_base.unwrap_or(raw);
        if check.contains('*') {
            return Err(format!(
                "command policy grant {raw} may only use /** as its final suffix"
            ));
        }
        match recursive_base {
            Some("") => Err("command policy recursive grant base must not be empty".to_string()),
            Some(base) => CommandExecutablePath::normalize_absolute(base).map(Self::Recursive),
            None => CommandExecutablePath::normalize_absolute(raw).map(Self::Exact),
        }
    }

    pub(super) fn contains(&self, executable: &Path) -> bool {
        match self {
            Self::Exact(path) => path == executable,
            Self::Recursive(base) => executable.starts_with(base),
        }
    }

    fn bound_contains(&self, raw_scope: &str, executable: &Path) -> Result<bool, String> {
        match self {
            Self::Exact(_) => Ok(PolicyPath::bind(Path::new(raw_scope))? == executable),
            Self::Recursive(_) => Ok(executable.starts_with(PolicyPath::bind(Path::new(
                raw_scope
                    .strip_suffix("/**")
                    .expect("recursive grant scope"),
            ))?)),
        }
    }
}

impl CommandRuleDraftValidator {
    pub(super) fn validate_bound_grant(
        grants: &[CommandPolicyRulesApplyGrant],
        rule: &StoredCommandRule,
    ) -> Result<(), String> {
        for grant in grants
            .iter()
            .filter(|grant| grant.decision == rule.decision)
        {
            if CommandGrantScope::parse(&grant.path_scope).is_ok_and(|scope| {
                scope.contains(&rule.executable)
                    && scope
                        .bound_contains(&grant.path_scope, &rule.execution_executable)
                        .unwrap_or(false)
            }) {
                return Ok(());
            }
        }
        Err(format!(
            "missing command-policy.rules.apply grant for bound target {} of {}",
            rule.execution_executable.display(),
            rule.executable.display(),
        ))
    }

    pub(super) fn validate_request(
        revision: u64,
        owner_instance_id: &str,
        request: &CommandPolicyApplyRequest,
    ) -> Vec<CommandPolicyApplyError> {
        let mut errors = Vec::new();
        if owner_instance_id.trim().is_empty() || owner_instance_id == STATIC_POLICY_OWNER {
            errors.push(Self::apply_error(
                0,
                "invalid-owner",
                "command policy owner must be a non-static plugin instance id",
            ));
        }
        if request.base_revision != revision {
            errors.push(Self::apply_error(
                0,
                "revision-conflict",
                format!(
                    "command policy base revision {} does not match current revision {revision}",
                    request.base_revision
                ),
            ));
        }
        if request.mutation_id.trim().is_empty() {
            errors.push(Self::apply_error(
                0,
                "invalid-mutation-id",
                "command policy mutation_id must not be empty",
            ));
        }
        if request.items.is_empty() {
            errors.push(Self::apply_error(
                0,
                "empty-mutation",
                "command policy apply requires at least one item",
            ));
        }
        errors
    }

    pub(super) fn validate_owner_rules(
        owner_instance_id: &str,
        rules: &BTreeMap<String, StoredCommandRule>,
    ) -> Result<(), String> {
        let mut scopes = BTreeSet::new();
        for (key, rule) in rules {
            if key != &rule.rule_id {
                return Err(format!(
                    "command policy owner {owner_instance_id} has inconsistent rule id {key}"
                ));
            }
            if !scopes.insert((rule.executable.clone(), rule.args.logical_scope())) {
                return Err(format!(
                    "command policy owner {owner_instance_id} has duplicate executable {} args {}",
                    rule.executable.display(),
                    rule.args.describe(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_draft_grant(
        grants: &[CommandPolicyRulesApplyGrant],
        draft: &CommandPolicyRuleDraft,
    ) -> Result<(), String> {
        let executable = CommandExecutablePath::normalize_absolute(&draft.executable)?;
        if grants.iter().any(|grant| {
            grant.decision == draft.decision
                && CommandGrantScope::parse(&grant.path_scope)
                    .is_ok_and(|scope| scope.contains(&executable))
        }) {
            return Ok(());
        }
        Err(format!(
            "missing command-policy.rules.apply grant for {} {}",
            draft.decision.as_str(),
            executable.display()
        ))
    }

    pub(super) fn accepted_result(revision: u64, applied_count: u32) -> CommandPolicyApplyResult {
        CommandPolicyApplyResult {
            status: CommandPolicyApplyStatus::Accepted,
            new_revision: revision,
            applied_count,
            rejected_count: 0,
            errors: Vec::new(),
        }
    }

    pub(super) fn rejected_result(
        revision: u64,
        errors: Vec<CommandPolicyApplyError>,
    ) -> CommandPolicyApplyResult {
        CommandPolicyApplyResult {
            status: CommandPolicyApplyStatus::Rejected,
            new_revision: revision,
            applied_count: 0,
            rejected_count: errors.len() as u32,
            errors,
        }
    }

    pub(super) fn apply_error(
        item_index: u32,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> CommandPolicyApplyError {
        CommandPolicyApplyError {
            item_index,
            code: code.into(),
            message: message.into(),
        }
    }
}
