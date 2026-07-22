use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use config_core::daemon::EnforcementDecision;
use plugin_system::{
    FilePolicyApplyError, FilePolicyApplyResult, FilePolicyApplyStatus, FilePolicyDecision,
    FilePolicyListFilter, FilePolicyOperation, FilePolicyRuleDraft, FilePolicyRuleView,
    FilePolicyRulesApplyGrant,
};

use super::scope::{PathScope, canonical_exact_path, normalized_scope};
use super::{
    DEFAULT_GRAY_CONCURRENCY_LIMIT, DEFAULT_GRAY_FALLBACK, DEFAULT_GRAY_TIMEOUT_MS,
    EnforcementRule, HOST_RULE_ID_PREFIX, RuleDecision,
};

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
        let rule = EnforcementRule {
            owner_instance_id: owner_instance_id.clone(),
            rule_id: rule_id.clone(),
            decision: rule_decision(decision, gray_target)?,
            operation: operation.as_str().to_string(),
            path: scope.display_base(),
        };
        Ok(Self {
            owner_instance_id,
            rule_id,
            tier,
            identity,
            decision,
            operation,
            scope,
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
            normalized_scope(&draft.path)?,
            draft.gray_target,
            draft.priority,
            true,
            updated_revision,
            updated_sequence,
        )
    }

    pub(super) fn matches_path(&self, operation: FilePolicyOperation, path: &Path) -> bool {
        self.operation.matches(operation) && self.scope.matches_path(path)
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

    pub(super) fn validate_mark_directories(&self) -> Result<(), String> {
        if !self.contributes_mark_directories() {
            return Ok(());
        }
        self.scope
            .collect_mark_directories(&mut BTreeSet::<PathBuf>::new())
            .map_err(|error| {
                format!(
                    "operation {} requires fanotify open coverage: {error}",
                    self.operation.as_str()
                )
            })
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
}

pub(super) fn parse_rule(raw: &str) -> Result<EnforcementRule, String> {
    let fields = raw.split_whitespace().collect::<Vec<_>>();
    match fields.as_slice() {
        [rule_id, decision, operation, path] => parse_local_rule(rule_id, decision, operation, path),
        [
            rule_id,
            "gray",
            "sync-plugin",
            instance_id,
            "timeout-ms",
            timeout_ms,
            "concurrency",
            concurrency_limit,
            "fallback",
            fallback,
            operation,
            path,
        ] => parse_sync_plugin_rule(
            rule_id,
            instance_id,
            timeout_ms,
            concurrency_limit,
            fallback,
            operation,
            path,
        ),
        _ => Err(
            "expected: <rule_id> <allow|deny> <any|open|mkdir|rmdir> <absolute-path> or <rule_id> gray sync-plugin <instance> timeout-ms <positive-ms> concurrency <positive-limit> fallback <allow|deny> <any|open|mkdir|rmdir> <absolute-path>"
                .to_string(),
        ),
    }
}

pub(super) fn validate_draft_grant(
    grants: &[FilePolicyRulesApplyGrant],
    draft: &FilePolicyRuleDraft,
) -> Result<(), String> {
    let scope = normalized_scope(&draft.path)?;
    validate_rule_grant(grants, draft.decision, &scope)
}

pub(super) fn validate_rule_grant(
    grants: &[FilePolicyRulesApplyGrant],
    decision: FilePolicyDecision,
    scope: &PathScope,
) -> Result<(), String> {
    if grants.iter().any(|grant| {
        grant.decision == decision
            && normalized_scope(&grant.path_scope)
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

pub(super) fn validate_gray_target<F>(
    draft: &FilePolicyRuleDraft,
    gray_target_exists: &F,
) -> Result<(), String>
where
    F: Fn(u64) -> bool,
{
    if draft.decision != FilePolicyDecision::Gray {
        return Ok(());
    }
    let target = draft
        .gray_target
        .ok_or_else(|| "gray rule requires gray_target".to_string())?;
    if !gray_target_exists(target) {
        return Err(format!(
            "gray_target {target} is not an active control decider"
        ));
    }
    Ok(())
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

fn parse_local_rule(
    rule_id: &str,
    decision: &str,
    operation: &str,
    path: &str,
) -> Result<EnforcementRule, String> {
    let operation = FilePolicyOperation::from_wire(operation)?;
    Ok(EnforcementRule {
        owner_instance_id: super::STATIC_POLICY_OWNER.to_string(),
        rule_id: rule_id.to_string(),
        decision: RuleDecision::Local(decision.parse::<EnforcementDecision>()?),
        operation: operation.as_str().to_string(),
        path: canonical_exact_path(path)?,
    })
}

fn parse_sync_plugin_rule(
    rule_id: &str,
    instance_id: &str,
    timeout_ms: &str,
    concurrency_limit: &str,
    fallback: &str,
    operation: &str,
    path: &str,
) -> Result<EnforcementRule, String> {
    let operation = FilePolicyOperation::from_wire(operation)?;
    if instance_id.trim().is_empty() {
        return Err("sync-plugin instance id must not be empty".to_string());
    }
    Ok(EnforcementRule {
        owner_instance_id: super::STATIC_POLICY_OWNER.to_string(),
        rule_id: rule_id.to_string(),
        decision: RuleDecision::SyncPlugin {
            instance_id: instance_id.to_string(),
            timeout_ms: parse_positive_u64("timeout-ms", timeout_ms)?,
            concurrency_limit: parse_positive_u32("concurrency", concurrency_limit)?,
            fallback: fallback.parse::<EnforcementDecision>()?,
        },
        operation: operation.as_str().to_string(),
        path: canonical_exact_path(path)?,
    })
}

fn rule_decision(
    decision: FilePolicyDecision,
    gray_target: Option<u64>,
) -> Result<RuleDecision, String> {
    match decision {
        FilePolicyDecision::Allow => Ok(RuleDecision::Local(EnforcementDecision::Allow)),
        FilePolicyDecision::Deny => Ok(RuleDecision::Local(EnforcementDecision::Deny)),
        FilePolicyDecision::Gray => {
            let Some(target) = gray_target else {
                return Err("gray rule requires gray_target".to_string());
            };
            Ok(RuleDecision::SyncPlugin {
                instance_id: target.to_string(),
                timeout_ms: DEFAULT_GRAY_TIMEOUT_MS,
                concurrency_limit: DEFAULT_GRAY_CONCURRENCY_LIMIT,
                fallback: DEFAULT_GRAY_FALLBACK,
            })
        }
        FilePolicyDecision::Default => {
            Err("file policy rule decision cannot be default".to_string())
        }
    }
}

fn parse_positive_u64(name: &str, raw: &str) -> Result<u64, String> {
    let value = raw
        .parse::<u64>()
        .map_err(|error| format!("{name} must be a positive integer: {error}"))?;
    if value == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(value)
}

fn parse_positive_u32(name: &str, raw: &str) -> Result<u32, String> {
    let value = raw
        .parse::<u32>()
        .map_err(|error| format!("{name} must be a positive integer: {error}"))?;
    if value == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(value)
}
