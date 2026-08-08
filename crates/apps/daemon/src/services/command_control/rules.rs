//! Static and runtime-owned command policy storage.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use plugin_system::{
    CommandPolicyApplyError, CommandPolicyApplyRequest, CommandPolicyApplyResult,
    CommandPolicyApplyStatus, CommandPolicyDecision, CommandPolicyListFilter,
    CommandPolicyListResult, CommandPolicyMatchDryRunRequest, CommandPolicyMatchDryRunResult,
    CommandPolicyPatchItem, CommandPolicyPatchOp, CommandPolicyRuleDraft, CommandPolicyRuleView,
    CommandPolicyRulesApplyGrant,
};

use super::decision::{CommandGrantScope, CommandPath, CommandRuleDraftValidator};

mod args;
mod parser;

use args::CommandArgsPattern;
use parser::StaticRuleParser;

pub(super) const STATIC_POLICY_OWNER: &str = "actrail.static";
const GENERATED_RULE_ID_PREFIX: &str = "command-rule";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StoredCommandRule {
    pub(super) owner_instance_id: String,
    pub(super) rule_id: String,
    pub(super) decision: CommandPolicyDecision,
    pub(super) executable: PathBuf,
    args: CommandArgsPattern,
    pub(super) gray_target: Option<String>,
    pub(super) priority: i32,
    pub(super) rule_revision: u64,
    pub(super) updated_sequence: u64,
}

impl StoredCommandRule {
    pub(super) fn args_view(&self) -> Option<Vec<String>> {
        self.args.view()
    }

    fn from_draft(
        owner_instance_id: &str,
        item_rule_id: Option<&str>,
        draft: &CommandPolicyRuleDraft,
        rule_revision: u64,
        updated_sequence: u64,
        generated_rule_id: u64,
    ) -> Result<Self, String> {
        let rule_id = item_rule_id
            .map(str::to_string)
            .or_else(|| draft.rule_id.clone())
            .unwrap_or_else(|| format!("{GENERATED_RULE_ID_PREFIX}-{generated_rule_id}"));
        CommandRuleDraftValidator::validate_id(&rule_id)?;
        CommandRuleDraftValidator::validate_shape(draft)?;
        Ok(Self {
            owner_instance_id: owner_instance_id.to_string(),
            rule_id,
            decision: draft.decision,
            executable: CommandPath::normalize_absolute(&draft.executable)?,
            args: CommandArgsPattern::parse(draft.args.as_deref())?,
            gray_target: draft.gray_target.clone(),
            priority: draft.priority,
            rule_revision,
            updated_sequence,
        })
    }

    fn view(&self) -> CommandPolicyRuleView {
        CommandPolicyRuleView {
            rule_id: self.rule_id.clone(),
            owner_instance_id: self.owner_instance_id.clone(),
            decision: self.decision,
            executable: self.executable.display().to_string(),
            args: self.args.view(),
            gray_target: self.gray_target.clone(),
            priority: self.priority,
            rule_revision: self.rule_revision,
            updated_sequence: self.updated_sequence,
        }
    }

    fn matches_filter(&self, filter: &CommandPolicyListFilter) -> bool {
        filter
            .decision
            .is_none_or(|decision| decision == self.decision)
            && filter.executable_prefix.as_ref().is_none_or(|prefix| {
                self.executable
                    .display()
                    .to_string()
                    .starts_with(prefix.as_str())
            })
    }
}

#[derive(Clone, Debug)]
pub(super) struct CommandPolicyStore {
    revision: u64,
    next_rule_id: u64,
    next_sequence: u64,
    by_owner: BTreeMap<String, BTreeMap<String, StoredCommandRule>>,
    effective_by_executable: BTreeMap<PathBuf, Vec<StoredCommandRule>>,
}

impl CommandPolicyStore {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(format!(
                    "read command control rules {} failed: {error}",
                    path.display()
                ));
            }
        };
        let mut store = Self::empty();
        let mut static_rules = BTreeMap::new();
        for (index, raw_line) in raw.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            store.next_sequence = store
                .next_sequence
                .checked_add(1)
                .ok_or_else(|| "command policy sequence overflow".to_string())?;
            let rule = StaticRuleParser::parse(line, store.next_sequence)
                .map_err(|message| format!("{}:{}: {message}", path.display(), index + 1))?;
            if static_rules.insert(rule.rule_id.clone(), rule).is_some() {
                return Err(format!(
                    "{}:{}: duplicate command rule id",
                    path.display(),
                    index + 1
                ));
            }
        }
        validate_owner_rules(STATIC_POLICY_OWNER, &static_rules)?;
        if !static_rules.is_empty() {
            store.revision = 1;
            for rule in static_rules.values_mut() {
                rule.rule_revision = store.revision;
            }
            store
                .by_owner
                .insert(STATIC_POLICY_OWNER.to_string(), static_rules);
            store.rebuild_effective();
        }
        Ok(store)
    }

    fn empty() -> Self {
        Self {
            revision: 0,
            next_rule_id: 1,
            next_sequence: 0,
            by_owner: BTreeMap::new(),
            effective_by_executable: BTreeMap::new(),
        }
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn find(&self, executable: &Path, args: &[String]) -> Option<&StoredCommandRule> {
        self.effective_by_executable
            .get(executable)
            .and_then(|rules| rules.iter().find(|rule| rule.args.matches(args)))
    }

    pub(super) fn requires_args(&self, executable: &Path) -> bool {
        self.effective_by_executable
            .get(executable)
            .is_some_and(|rules| rules.iter().any(|rule| rule.args.requires_snapshot()))
    }

    pub(super) fn is_rule_current(&self, rule: &StoredCommandRule) -> bool {
        self.by_owner
            .get(&rule.owner_instance_id)
            .and_then(|rules| rules.get(&rule.rule_id))
            .is_some_and(|current| current.rule_revision == rule.rule_revision)
    }

    pub(super) fn list(
        &self,
        filter: CommandPolicyListFilter,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<CommandPolicyListResult, String> {
        if limit == 0 {
            return Err("command policy list limit must be positive".to_string());
        }
        let start = cursor
            .map(|raw| {
                raw.parse::<usize>()
                    .map_err(|error| format!("invalid command policy cursor {raw}: {error}"))
            })
            .transpose()?
            .unwrap_or(0);
        let limit = usize::try_from(limit)
            .map_err(|error| format!("command policy list limit overflow: {error}"))?;
        let mut views = self
            .by_owner
            .values()
            .flat_map(|rules| rules.values())
            .filter(|rule| rule.matches_filter(&filter))
            .map(StoredCommandRule::view)
            .collect::<Vec<_>>();
        views.sort_by(|left, right| {
            left.owner_instance_id
                .cmp(&right.owner_instance_id)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
        });
        let total = views.len();
        let rules = views
            .into_iter()
            .skip(start)
            .take(limit)
            .collect::<Vec<_>>();
        let next = start.saturating_add(rules.len());
        Ok(CommandPolicyListResult {
            rules,
            next_cursor: (next < total).then(|| next.to_string()),
            source_revision: self.revision,
        })
    }

    pub(super) fn match_dry_run(
        &self,
        request: CommandPolicyMatchDryRunRequest,
        default_decision: CommandPolicyDecision,
    ) -> Result<CommandPolicyMatchDryRunResult, String> {
        let executable = CommandPath::normalize_absolute(&request.executable)?;
        let matched = self.find(&executable, &request.args);
        Ok(CommandPolicyMatchDryRunResult {
            matched: matched.is_some(),
            decision: matched
                .map(|rule| rule.decision)
                .unwrap_or(default_decision),
            rule_id: matched.map(|rule| rule.rule_id.clone()),
            owner_instance_id: matched.map(|rule| rule.owner_instance_id.clone()),
            resolved_executable: executable.display().to_string(),
            rule_revision: matched.map(|rule| rule.rule_revision),
            source_revision: self.revision,
        })
    }

    pub(super) fn validate_apply<F>(
        &self,
        owner_instance_id: &str,
        grants: &[CommandPolicyRulesApplyGrant],
        request: &CommandPolicyApplyRequest,
        target_active: F,
    ) -> CommandPolicyApplyResult
    where
        F: Fn(&str) -> bool,
    {
        match self.candidate_after(owner_instance_id, grants, request, target_active) {
            Ok(_) => accepted_result(self.revision, request.items.len() as u32),
            Err(errors) => rejected_result(self.revision, errors),
        }
    }

    pub(super) fn apply<F>(
        &mut self,
        owner_instance_id: &str,
        grants: &[CommandPolicyRulesApplyGrant],
        request: CommandPolicyApplyRequest,
        target_active: F,
    ) -> CommandPolicyApplyResult
    where
        F: Fn(&str) -> bool,
    {
        let applied_count = request.items.len() as u32;
        match self.candidate_after(owner_instance_id, grants, &request, target_active) {
            Ok(candidate) => {
                *self = candidate;
                accepted_result(self.revision, applied_count)
            }
            Err(errors) => rejected_result(self.revision, errors),
        }
    }

    pub(super) fn remove_owner(&mut self, owner_instance_id: &str) -> Result<bool, String> {
        if owner_instance_id == STATIC_POLICY_OWNER {
            return Err("static command policy owner cannot be removed".to_string());
        }
        if self.by_owner.remove(owner_instance_id).is_none() {
            return Ok(false);
        }
        self.bump_revision()?;
        self.rebuild_effective();
        Ok(true)
    }

    fn candidate_after<F>(
        &self,
        owner_instance_id: &str,
        grants: &[CommandPolicyRulesApplyGrant],
        request: &CommandPolicyApplyRequest,
        target_active: F,
    ) -> Result<Self, Vec<CommandPolicyApplyError>>
    where
        F: Fn(&str) -> bool,
    {
        let mut request_errors = validate_request(self.revision, owner_instance_id, request);
        if !request_errors.is_empty() {
            return Err(request_errors);
        }
        let next_revision = match self.revision.checked_add(1) {
            Some(revision) => revision,
            None => {
                return Err(vec![apply_error(
                    0,
                    "revision-overflow",
                    "command policy revision overflow",
                )]);
            }
        };
        let mut candidate = self.clone();
        let mut owner_rules = candidate
            .by_owner
            .remove(owner_instance_id)
            .unwrap_or_default();
        for (index, item) in request.items.iter().enumerate() {
            if let Err(message) = candidate.apply_item(
                owner_instance_id,
                &mut owner_rules,
                item,
                next_revision,
                &target_active,
                grants,
            ) {
                request_errors.push(apply_error(index as u32, "invalid-rule", message));
            }
        }
        if request_errors.is_empty() {
            if let Err(message) = validate_owner_rules(owner_instance_id, &owner_rules) {
                request_errors.push(apply_error(0, "duplicate-scope", message));
            }
        }
        if !request_errors.is_empty() {
            return Err(request_errors);
        }
        if owner_rules.is_empty() {
            candidate.by_owner.remove(owner_instance_id);
        } else {
            candidate
                .by_owner
                .insert(owner_instance_id.to_string(), owner_rules);
        }
        candidate.revision = next_revision;
        candidate.rebuild_effective();
        Ok(candidate)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_item<F>(
        &mut self,
        owner_instance_id: &str,
        owner_rules: &mut BTreeMap<String, StoredCommandRule>,
        item: &CommandPolicyPatchItem,
        next_revision: u64,
        target_active: &F,
        grants: &[CommandPolicyRulesApplyGrant],
    ) -> Result<(), String>
    where
        F: Fn(&str) -> bool,
    {
        match item.op {
            CommandPolicyPatchOp::Delete => {
                if item.rule.is_some() {
                    return Err("delete item must not include rule".to_string());
                }
                let rule_id = item
                    .rule_id
                    .as_deref()
                    .ok_or_else(|| "delete item requires rule_id".to_string())?;
                owner_rules
                    .remove(rule_id)
                    .ok_or_else(|| format!("command rule {rule_id} not found for owner"))?;
                Ok(())
            }
            CommandPolicyPatchOp::Upsert => {
                let draft = item
                    .rule
                    .as_ref()
                    .ok_or_else(|| "upsert item requires rule".to_string())?;
                if let (Some(item_id), Some(draft_id)) =
                    (item.rule_id.as_deref(), draft.rule_id.as_deref())
                    && item_id != draft_id
                {
                    return Err(format!(
                        "item rule_id {item_id} does not match draft rule_id {draft_id}"
                    ));
                }
                validate_draft_grant(grants, draft)?;
                if draft.decision == CommandPolicyDecision::Gray {
                    let target = draft
                        .gray_target
                        .as_deref()
                        .ok_or_else(|| "gray command rule requires gray_target".to_string())?;
                    if target == owner_instance_id {
                        return Err("gray command rule cannot target its policy owner".to_string());
                    }
                    if !target_active(target) {
                        return Err(format!(
                            "gray_target {target} is not an active control decider"
                        ));
                    }
                }
                self.next_sequence = self
                    .next_sequence
                    .checked_add(1)
                    .ok_or_else(|| "command policy sequence overflow".to_string())?;
                let generated_rule_id = self.next_rule_id;
                let rule = StoredCommandRule::from_draft(
                    owner_instance_id,
                    item.rule_id.as_deref(),
                    draft,
                    next_revision,
                    self.next_sequence,
                    generated_rule_id,
                )?;
                if item.rule_id.is_none() && draft.rule_id.is_none() {
                    self.next_rule_id = self
                        .next_rule_id
                        .checked_add(1)
                        .ok_or_else(|| "command policy generated rule id overflow".to_string())?;
                }
                owner_rules.insert(rule.rule_id.clone(), rule);
                Ok(())
            }
        }
    }

    fn bump_revision(&mut self) -> Result<(), String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| "command policy revision overflow".to_string())?;
        Ok(())
    }

    fn rebuild_effective(&mut self) {
        let mut effective = BTreeMap::<PathBuf, Vec<StoredCommandRule>>::new();
        for rule in self.by_owner.values().flat_map(|rules| rules.values()) {
            effective
                .entry(rule.executable.clone())
                .or_default()
                .push(rule.clone());
        }
        for rules in effective.values_mut() {
            rules.sort_by(|left, right| {
                right
                    .priority
                    .cmp(&left.priority)
                    .then_with(|| right.updated_sequence.cmp(&left.updated_sequence))
                    .then_with(|| left.owner_instance_id.cmp(&right.owner_instance_id))
                    .then_with(|| left.rule_id.cmp(&right.rule_id))
            });
        }
        self.effective_by_executable = effective;
    }
}

fn validate_request(
    revision: u64,
    owner_instance_id: &str,
    request: &CommandPolicyApplyRequest,
) -> Vec<CommandPolicyApplyError> {
    let mut errors = Vec::new();
    if owner_instance_id.trim().is_empty() || owner_instance_id == STATIC_POLICY_OWNER {
        errors.push(apply_error(
            0,
            "invalid-owner",
            "command policy owner must be a non-static plugin instance id",
        ));
    }
    if request.base_revision != revision {
        errors.push(apply_error(
            0,
            "revision-conflict",
            format!(
                "command policy base revision {} does not match current revision {revision}",
                request.base_revision
            ),
        ));
    }
    if request.mutation_id.trim().is_empty() {
        errors.push(apply_error(
            0,
            "invalid-mutation-id",
            "command policy mutation_id must not be empty",
        ));
    }
    if request.items.is_empty() {
        errors.push(apply_error(
            0,
            "empty-mutation",
            "command policy apply requires at least one item",
        ));
    }
    errors
}

fn validate_owner_rules(
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

fn validate_draft_grant(
    grants: &[CommandPolicyRulesApplyGrant],
    draft: &CommandPolicyRuleDraft,
) -> Result<(), String> {
    let executable = CommandPath::normalize_absolute(&draft.executable)?;
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

fn accepted_result(revision: u64, applied_count: u32) -> CommandPolicyApplyResult {
    CommandPolicyApplyResult {
        status: CommandPolicyApplyStatus::Accepted,
        new_revision: revision,
        applied_count,
        rejected_count: 0,
        errors: Vec::new(),
    }
}

fn rejected_result(
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

fn apply_error(
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
