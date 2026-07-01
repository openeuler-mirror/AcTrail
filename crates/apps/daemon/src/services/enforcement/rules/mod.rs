//! File-access enforcement rule store.

mod model;
mod scope;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use config_core::daemon::EnforcementDecision;
use plugin_system::{
    FilePolicyApplyError, FilePolicyApplyMode, FilePolicyApplyPrecondition, FilePolicyApplyRequest,
    FilePolicyApplyResult, FilePolicyApplyStatus, FilePolicyDecision, FilePolicyListFilter,
    FilePolicyListResult, FilePolicyMatchDryRunRequest, FilePolicyMatchDryRunResult,
    FilePolicyOperation, FilePolicyPatchItem, FilePolicyPatchOp, FilePolicyRulesApplyGrant,
};

use self::model::{
    RuleIdentityKind, RuleTier, StoredPolicyRule, apply_result_from_errors, parse_rule,
    validate_draft_grant, validate_gray_target, validate_rule_grant,
};
use self::scope::{PathScope, absolute_scope, normalized_scope};

const BUILTIN_POLICY_OWNER: &str = "actrail.builtin";
const STATIC_POLICY_OWNER: &str = "actrail.static";
const HOST_RULE_ID_PREFIX: &str = "fp";
const DEFAULT_GRAY_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_GRAY_CONCURRENCY_LIMIT: u32 = 32;
const DEFAULT_GRAY_FALLBACK: EnforcementDecision = EnforcementDecision::Deny;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct FileKey {
    pub dev: u64,
    pub ino: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EnforcementRule {
    pub rule_id: String,
    pub decision: RuleDecision,
    pub operation: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BuiltinIgnoreMark {
    pub path: PathBuf,
    pub recursive: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum RuleDecision {
    Local(EnforcementDecision),
    SyncPlugin {
        instance_id: String,
        timeout_ms: u64,
        concurrency_limit: u32,
        fallback: EnforcementDecision,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EnforcementRules {
    revision: u64,
    next_rule_id: u64,
    next_sequence: u64,
    by_owner: BTreeMap<String, BTreeMap<String, StoredPolicyRule>>,
    effective_rules: Vec<StoredPolicyRule>,
    mark_directories: BTreeSet<PathBuf>,
}

impl EnforcementRules {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("read enforcement rules {}: {error}", path.display()))?;
        Self::parse(&raw)
    }

    pub(super) fn parse(raw: &str) -> Result<Self, String> {
        let mut store = Self::empty();
        let mut rule_ids = BTreeSet::new();
        for (line_index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let rule = parse_rule(trimmed)
                .map_err(|error| format!("enforcement rule line {}: {error}", line_index + 1))?;
            if !rule_ids.insert(rule.rule_id.clone()) {
                return Err(format!(
                    "enforcement rule line {} duplicates rule id {}",
                    line_index + 1,
                    rule.rule_id
                ));
            }
            store.insert_static_rule(rule)?;
        }
        store.rebuild_effective()?;
        Ok(store)
    }

    pub(super) fn find_path(&self, path: &Path) -> Option<&EnforcementRule> {
        self.effective_rules
            .iter()
            .find(|rule| rule.matches_path(path))
            .map(|rule| &rule.rule)
    }

    pub(super) fn find_builtin_allow(&self, path: &Path) -> Option<&EnforcementRule> {
        self.effective_rules
            .iter()
            .find(|rule| rule.is_builtin_allow() && rule.matches_path(path))
            .map(|rule| &rule.rule)
    }

    pub(super) fn install_builtin_allow_rules<I>(&mut self, rules: I) -> Result<(), String>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        for (rule_id, path) in rules {
            self.insert_builtin_allow_rule(rule_id, path)?;
        }
        self.rebuild_effective()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.effective_rules.is_empty()
    }

    pub(super) fn mark_directories(&self) -> impl Iterator<Item = &PathBuf> {
        self.mark_directories.iter()
    }

    pub(super) fn builtin_ignore_marks(&self) -> Vec<BuiltinIgnoreMark> {
        self.effective_rules
            .iter()
            .filter(|rule| rule.is_builtin_allow())
            .map(|rule| BuiltinIgnoreMark {
                path: rule.scope.display_base(),
                recursive: rule.scope.is_recursive(),
            })
            .collect()
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn list(
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

    pub(super) fn match_dry_run(
        &self,
        request: FilePolicyMatchDryRunRequest,
        default_decision: FilePolicyDecision,
    ) -> Result<FilePolicyMatchDryRunResult, String> {
        let path = normalized_scope(&request.path)?;
        let canonical_path = path.display_path();
        let matched = self
            .effective_rules
            .iter()
            .find(|rule| rule.operation == request.operation && rule.scope.contains_scope(&path));
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

    pub(super) fn validate_apply<F>(
        &self,
        owner_instance_id: &str,
        grants: &[FilePolicyRulesApplyGrant],
        request: &FilePolicyApplyRequest,
        gray_target_exists: F,
    ) -> FilePolicyApplyResult
    where
        F: Fn(u64) -> bool,
    {
        if let Some(result) = self.reject_invalid_precondition(&request.precondition) {
            return result;
        }
        let errors = self.validate_apply_items(
            owner_instance_id,
            grants,
            request.precondition.base_revision,
            &request.items,
            &gray_target_exists,
        );
        apply_result_from_errors(self.revision, 0, errors)
    }

    pub(super) fn apply<F>(
        &mut self,
        owner_instance_id: &str,
        grants: &[FilePolicyRulesApplyGrant],
        request: FilePolicyApplyRequest,
        gray_target_exists: F,
    ) -> Result<(FilePolicyApplyResult, Vec<PathBuf>), String>
    where
        F: Fn(u64) -> bool,
    {
        if let Some(result) = self.reject_invalid_precondition(&request.precondition) {
            return Ok((result, Vec::new()));
        }
        let errors = self.validate_apply_items(
            owner_instance_id,
            grants,
            request.precondition.base_revision,
            &request.items,
            &gray_target_exists,
        );

        let mut applied = 0_u32;
        let rejected = errors.len() as u32;
        let invalid_indexes = errors
            .iter()
            .map(|error| error.item_index)
            .collect::<BTreeSet<_>>();
        for (index, item) in request.items.into_iter().enumerate() {
            if invalid_indexes.contains(&(index as u32)) {
                continue;
            }
            let mutation_revision = self.revision.saturating_add(1);
            self.apply_item(owner_instance_id, item, mutation_revision)?;
            applied = applied.saturating_add(1);
        }
        if applied > 0 {
            self.bump_revision()?;
        }
        let status = if errors.is_empty() || applied > 0 {
            FilePolicyApplyStatus::Accepted
        } else {
            FilePolicyApplyStatus::Rejected
        };
        Ok((
            FilePolicyApplyResult {
                status,
                new_revision: self.revision,
                applied_count: applied,
                rejected_count: rejected,
                errors,
            },
            self.mark_directories.iter().cloned().collect(),
        ))
    }

    pub(super) fn remove_owner(&mut self, owner_instance_id: &str) -> Result<Vec<PathBuf>, String> {
        if self.by_owner.remove(owner_instance_id).is_none() {
            return Ok(Vec::new());
        }
        self.bump_revision()?;
        Ok(self.mark_directories.iter().cloned().collect())
    }

    fn reject_invalid_precondition(
        &self,
        precondition: &FilePolicyApplyPrecondition,
    ) -> Option<FilePolicyApplyResult> {
        if precondition.apply_mode == FilePolicyApplyMode::Aon {
            return Some(apply_result_from_errors(
                self.revision,
                0,
                vec![FilePolicyApplyError {
                    item_index: 0,
                    code: "unsupported-apply-mode".to_string(),
                    message: "file policy apply_mode aon is temporarily disabled".to_string(),
                }],
            ));
        }
        if precondition.base_revision > self.revision {
            return Some(apply_result_from_errors(
                self.revision,
                0,
                vec![FilePolicyApplyError {
                    item_index: 0,
                    code: "invalid-precondition".to_string(),
                    message: format!(
                        "base_revision {} is newer than current revision {}",
                        precondition.base_revision, self.revision
                    ),
                }],
            ));
        }
        None
    }

    fn empty() -> Self {
        Self {
            revision: 0,
            next_rule_id: 1,
            next_sequence: 1,
            by_owner: BTreeMap::new(),
            effective_rules: Vec::new(),
            mark_directories: BTreeSet::new(),
        }
    }

    fn insert_static_rule(&mut self, rule: EnforcementRule) -> Result<(), String> {
        let decision = match &rule.decision {
            RuleDecision::Local(EnforcementDecision::Allow) => FilePolicyDecision::Allow,
            RuleDecision::Local(EnforcementDecision::Deny) => FilePolicyDecision::Deny,
            RuleDecision::SyncPlugin { .. } => FilePolicyDecision::Gray,
        };
        let stored = StoredPolicyRule {
            owner_instance_id: STATIC_POLICY_OWNER.to_string(),
            rule_id: rule.rule_id.clone(),
            tier: RuleTier::Static,
            identity: RuleIdentityKind::Static,
            decision,
            operation: FilePolicyOperation::Open,
            scope: PathScope::Exact(rule.path.clone()),
            gray_target: None,
            priority: 0,
            enabled: true,
            updated_revision: self.revision,
            updated_sequence: self.next_sequence(),
            rule,
        };
        self.by_owner
            .entry(STATIC_POLICY_OWNER.to_string())
            .or_default()
            .insert(stored.rule_id.clone(), stored);
        Ok(())
    }

    fn insert_builtin_allow_rule(&mut self, rule_id: String, path: String) -> Result<(), String> {
        if rule_id.trim().is_empty() {
            return Err("builtin file policy rule_id must not be empty".to_string());
        }
        if self
            .by_owner
            .get(BUILTIN_POLICY_OWNER)
            .is_some_and(|rules| rules.contains_key(&rule_id))
        {
            return Err(format!("duplicate builtin file policy rule id {rule_id}"));
        }
        let scope = absolute_scope(&path)?;
        let rule = EnforcementRule {
            rule_id: rule_id.clone(),
            decision: RuleDecision::Local(EnforcementDecision::Allow),
            operation: FilePolicyOperation::Open.as_str().to_string(),
            path: scope.display_base(),
        };
        let stored = StoredPolicyRule {
            owner_instance_id: BUILTIN_POLICY_OWNER.to_string(),
            rule_id,
            tier: RuleTier::Builtin,
            identity: RuleIdentityKind::Builtin,
            decision: FilePolicyDecision::Allow,
            operation: FilePolicyOperation::Open,
            scope,
            gray_target: None,
            priority: i32::MAX,
            enabled: true,
            updated_revision: self.revision,
            updated_sequence: self.next_sequence(),
            rule,
        };
        self.by_owner
            .entry(BUILTIN_POLICY_OWNER.to_string())
            .or_default()
            .insert(stored.rule_id.clone(), stored);
        Ok(())
    }

    fn validate_apply_items<F>(
        &self,
        owner_instance_id: &str,
        grants: &[FilePolicyRulesApplyGrant],
        base_revision: u64,
        items: &[FilePolicyPatchItem],
        gray_target_exists: &F,
    ) -> Vec<FilePolicyApplyError>
    where
        F: Fn(u64) -> bool,
    {
        let mut errors = Vec::new();
        for (index, item) in items.iter().enumerate() {
            if let Err(message) = self.validate_item(
                owner_instance_id,
                grants,
                base_revision,
                item,
                gray_target_exists,
            ) {
                errors.push(FilePolicyApplyError {
                    item_index: index as u32,
                    code: "invalid-rule".to_string(),
                    message,
                });
            }
        }
        errors
    }

    fn validate_item<F>(
        &self,
        owner_instance_id: &str,
        grants: &[FilePolicyRulesApplyGrant],
        base_revision: u64,
        item: &FilePolicyPatchItem,
        gray_target_exists: &F,
    ) -> Result<(), String>
    where
        F: Fn(u64) -> bool,
    {
        match item.op {
            FilePolicyPatchOp::Upsert => {
                let draft = item
                    .rule
                    .as_ref()
                    .ok_or_else(|| "upsert requires rule".to_string())?;
                validate_draft_grant(grants, draft)?;
                validate_gray_target(draft, gray_target_exists)?;
                let draft_rule = StoredPolicyRule::from_draft(
                    owner_instance_id,
                    draft,
                    item.rule_id.as_deref(),
                    self.revision,
                    1,
                    self.next_rule_id,
                )?;
                self.validate_not_stale_upsert(owner_instance_id, base_revision, &draft_rule)?;
            }
            FilePolicyPatchOp::Delete | FilePolicyPatchOp::Enable | FilePolicyPatchOp::Disable => {
                let rule_id = item
                    .rule_id
                    .as_deref()
                    .ok_or_else(|| "rule_id is required".to_string())?;
                let existing = self
                    .by_owner
                    .get(owner_instance_id)
                    .and_then(|rules| rules.get(rule_id))
                    .ok_or_else(|| format!("rule {rule_id} not found for owner"))?;
                self.validate_not_stale_rule(base_revision, existing)?;
                validate_rule_grant(grants, existing.decision, &existing.scope)?;
            }
        }
        Ok(())
    }

    fn validate_not_stale_upsert(
        &self,
        owner_instance_id: &str,
        base_revision: u64,
        draft_rule: &StoredPolicyRule,
    ) -> Result<(), String> {
        let Some(owner_rules) = self.by_owner.get(owner_instance_id) else {
            return Ok(());
        };
        if let Some(existing) = owner_rules.get(&draft_rule.rule_id) {
            self.validate_not_stale_rule(base_revision, existing)?;
        }
        for existing in owner_rules.values() {
            if existing.has_same_logical_scope(draft_rule) {
                self.validate_not_stale_rule(base_revision, existing)?;
            }
        }
        Ok(())
    }

    fn validate_not_stale_rule(
        &self,
        base_revision: u64,
        existing: &StoredPolicyRule,
    ) -> Result<(), String> {
        if existing.updated_revision > base_revision {
            return Err(format!(
                "stale file policy request for rule {}: base_revision {} is older than rule revision {}",
                existing.rule_id, base_revision, existing.updated_revision
            ));
        }
        Ok(())
    }

    fn apply_item(
        &mut self,
        owner_instance_id: &str,
        item: FilePolicyPatchItem,
        mutation_revision: u64,
    ) -> Result<(), String> {
        match item.op {
            FilePolicyPatchOp::Upsert => {
                let draft = item
                    .rule
                    .ok_or_else(|| "upsert requires rule".to_string())?;
                let sequence = self.next_sequence();
                let next_rule_id = self.next_available_host_rule_number(owner_instance_id);
                let stored = StoredPolicyRule::from_draft(
                    owner_instance_id,
                    &draft,
                    item.rule_id.as_deref(),
                    mutation_revision,
                    sequence,
                    next_rule_id,
                )?;
                if stored.is_host_auto() {
                    self.next_rule_id = next_rule_id.saturating_add(1);
                }
                let owner_rules = self
                    .by_owner
                    .entry(owner_instance_id.to_string())
                    .or_default();
                if stored.is_host_auto() {
                    let superseded = owner_rules
                        .iter()
                        .filter_map(|(rule_id, existing)| {
                            (existing.is_host_auto()
                                && existing.has_same_logical_scope(&stored)
                                && existing.rule_id != stored.rule_id)
                                .then(|| rule_id.clone())
                        })
                        .collect::<Vec<_>>();
                    for rule_id in superseded {
                        if let Some(old_rule) = owner_rules.remove(&rule_id) {
                            tracing::info!(
                                owner = %owner_instance_id,
                                old_rule_id = %old_rule.rule_id,
                                new_rule_id = %stored.rule_id,
                                path = %stored.scope.display_path(),
                                operation = %stored.operation.as_str(),
                                reason = "auto-upsert-superseded",
                                "file policy rule superseded"
                            );
                        }
                    }
                }
                owner_rules.insert(stored.rule_id.clone(), stored);
            }
            FilePolicyPatchOp::Delete => {
                let Some(rule_id) = item.rule_id else {
                    return Err("rule_id is required".to_string());
                };
                if let Some(rules) = self.by_owner.get_mut(owner_instance_id) {
                    rules.remove(&rule_id);
                }
            }
            FilePolicyPatchOp::Enable | FilePolicyPatchOp::Disable => {
                let Some(rule_id) = item.rule_id else {
                    return Err("rule_id is required".to_string());
                };
                let sequence = self.next_sequence();
                let Some(rule) = self
                    .by_owner
                    .get_mut(owner_instance_id)
                    .and_then(|rules| rules.get_mut(&rule_id))
                else {
                    return Err(format!("rule {rule_id} not found for owner"));
                };
                rule.enabled = item.op == FilePolicyPatchOp::Enable;
                rule.updated_revision = mutation_revision;
                rule.updated_sequence = sequence;
            }
        }
        Ok(())
    }

    fn bump_revision(&mut self) -> Result<(), String> {
        self.revision = self.revision.saturating_add(1);
        self.rebuild_effective()
    }

    fn next_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }

    fn next_available_host_rule_number(&self, owner_instance_id: &str) -> u64 {
        let mut next = self.next_rule_id;
        while self
            .by_owner
            .get(owner_instance_id)
            .is_some_and(|rules| rules.contains_key(&format!("{HOST_RULE_ID_PREFIX}-{next}")))
        {
            next = next.saturating_add(1);
        }
        next
    }

    fn rebuild_effective(&mut self) -> Result<(), String> {
        let mut effective = self
            .by_owner
            .values()
            .flat_map(|rules| rules.values())
            .filter(|rule| rule.enabled)
            .cloned()
            .collect::<Vec<_>>();
        effective.sort_by(|left, right| {
            right
                .tier
                .rank()
                .cmp(&left.tier.rank())
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| right.updated_sequence.cmp(&left.updated_sequence))
        });
        let mut mark_directories = BTreeSet::new();
        for rule in &effective {
            if rule.contributes_mark_directories() {
                rule.scope.collect_mark_directories(&mut mark_directories)?;
            }
        }
        self.effective_rules = effective;
        self.mark_directories = mark_directories;
        Ok(())
    }
}
