use super::scope::PathScope;
use super::types::{EnforcementRule, HOST_RULE_ID_PREFIX, RuleDecision};
use plugin_system::{
    FilePolicyDecision, FilePolicyListFilter, FilePolicyOperation, FilePolicyRuleDraft,
    FilePolicyRuleView,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuleTier {
    Plugin,
    Static,
    Builtin,
}

impl RuleTier {
    pub(super) fn rank(self) -> u8 {
        match self {
            Self::Static | Self::Plugin => 0,
            Self::Builtin => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuleIdentityKind {
    HostAuto,
    Explicit,
    Static,
    Builtin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StoredPolicyRule {
    pub(super) owner_instance_id: String,
    pub(super) rule_id: String,
    pub(super) tier: RuleTier,
    pub(super) identity: RuleIdentityKind,
    pub(super) decision: FilePolicyDecision,
    pub(super) operation: FilePolicyOperation,
    pub(super) scope: PathScope,
    pub(super) execution_scope: PathScope,
    pub(super) mark_directories: BTreeSet<PathBuf>,
    pub(super) gray_target: Option<u64>,
    pub(super) priority: i32,
    pub(super) enabled: bool,
    pub(super) updated_revision: u64,
    pub(super) updated_sequence: u64,
    pub(super) rule: EnforcementRule,
}

impl StoredPolicyRule {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        owner_instance_id: String,
        rule_id: String,
        tier: RuleTier,
        identity: RuleIdentityKind,
        decision: FilePolicyDecision,
        operation: FilePolicyOperation,
        scope: PathScope,
        gray_target: Option<u64>,
        priority: i32,
        enabled: bool,
        updated_revision: u64,
        updated_sequence: u64,
    ) -> Result<Self, String> {
        if decision == FilePolicyDecision::Default {
            return Err("file policy rule decision cannot be default".to_string());
        }
        let execution_scope = scope.bind()?;
        let scope = PathScope::normalized_scope(&scope.display_path())?;
        let rule = EnforcementRule {
            owner_instance_id: owner_instance_id.clone(),
            rule_id: rule_id.clone(),
            decision: RuleDecision::from_policy(decision, gray_target)?,
            operation: operation.as_str().to_string(),
            path: execution_scope.display_base(),
        };
        Ok(Self {
            owner_instance_id,
            rule_id,
            tier,
            identity,
            decision,
            operation,
            scope,
            execution_scope,
            mark_directories: BTreeSet::new(),
            gray_target,
            priority,
            enabled,
            updated_revision,
            updated_sequence,
            rule,
        })
    }

    pub(super) fn from_draft(
        owner_instance_id: &str,
        draft: &FilePolicyRuleDraft,
        item_rule_id: Option<&str>,
        updated_revision: u64,
        updated_sequence: u64,
        next_rule_id: u64,
    ) -> Result<Self, String> {
        let rule_id = item_rule_id
            .map(str::to_string)
            .or_else(|| draft.rule_id.clone())
            .unwrap_or_else(|| format!("{HOST_RULE_ID_PREFIX}-{next_rule_id}"));
        if rule_id.trim().is_empty() {
            return Err("rule_id must not be empty".to_string());
        }
        let identity = if item_rule_id.is_some() || draft.rule_id.is_some() {
            RuleIdentityKind::Explicit
        } else {
            RuleIdentityKind::HostAuto
        };
        Self::new(
            owner_instance_id.to_string(),
            rule_id,
            RuleTier::Plugin,
            identity,
            draft.decision,
            draft.operation,
            PathScope::absolute_scope(&draft.path)?,
            draft.gray_target,
            draft.priority,
            true,
            updated_revision,
            updated_sequence,
        )
    }

    pub(super) fn matches_path(&self, operation: FilePolicyOperation, path: &Path) -> bool {
        self.operation.matches(operation) && self.execution_scope.matches_path(path)
    }

    pub(super) fn is_host_auto(&self) -> bool {
        self.identity == RuleIdentityKind::HostAuto
    }

    pub(super) fn has_same_logical_scope(&self, other: &Self) -> bool {
        self.owner_instance_id == other.owner_instance_id
            && self.operation == other.operation
            && self.scope == other.scope
    }

    pub(super) fn is_builtin_allow(&self) -> bool {
        self.tier == RuleTier::Builtin
            && self.decision == FilePolicyDecision::Allow
            && self.operation == FilePolicyOperation::Open
    }

    pub(super) fn contributes_mark_directories(&self) -> bool {
        self.tier != RuleTier::Builtin && self.operation.matches(FilePolicyOperation::Open)
    }

    pub(super) fn prepare_mark_directories(&mut self) -> Result<(), String> {
        self.mark_directories = self.collect_mark_directories()?;
        Ok(())
    }

    pub(super) fn collect_mark_directories(&self) -> Result<BTreeSet<PathBuf>, String> {
        if !self.contributes_mark_directories() {
            return Ok(BTreeSet::new());
        }
        let mut directories = BTreeSet::new();
        self.execution_scope
            .collect_mark_directories(&mut directories)
            .map_err(|error| {
                format!(
                    "operation {} requires fanotify open coverage: {error}",
                    self.operation.as_str()
                )
            })?;
        Ok(directories)
    }

    pub(super) fn matches_filter(&self, filter: &FilePolicyListFilter) -> bool {
        filter
            .decision
            .is_none_or(|decision| decision == self.decision)
            && filter
                .operation
                .is_none_or(|operation| operation == self.operation)
            && filter
                .path_prefix
                .as_ref()
                .is_none_or(|prefix| self.scope.display_path().starts_with(prefix))
    }

    pub(super) fn view(&self) -> FilePolicyRuleView {
        FilePolicyRuleView {
            rule_id: self.rule_id.clone(),
            owner_instance_id: self.owner_instance_id.clone(),
            decision: self.decision,
            operation: self.operation,
            path: self.scope.display_path(),
            gray_target: self.gray_target,
            priority: self.priority,
            enabled: self.enabled,
            updated_sequence: self.updated_sequence,
        }
    }

    pub(super) fn execution_view(&self) -> FilePolicyRuleView {
        FilePolicyRuleView {
            path: self.execution_scope.display_path(),
            ..self.view()
        }
    }
}
