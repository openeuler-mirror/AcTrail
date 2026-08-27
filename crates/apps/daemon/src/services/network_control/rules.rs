//! Static exact-endpoint and runtime selector network policy storage.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use config_core::daemon::EnforcementDecision;
use plugin_system::{
    ControlVerdict, NetworkPolicyApplyError, NetworkPolicyApplyRequest, NetworkPolicyApplyResult,
    NetworkPolicyApplyStatus, NetworkPolicyDecision, NetworkPolicyListFilter,
    NetworkPolicyListResult, NetworkPolicyMatchDryRunRequest, NetworkPolicyMatchDryRunResult,
    NetworkPolicyPatchItem, NetworkPolicyPatchOp, NetworkPolicyRemoteSelector,
    NetworkPolicyRuleDraft, NetworkPolicyRuleView, NetworkPolicyRulesApplyGrant,
};

use super::policy::{NetworkRuleValidator, StaticRuleParser};

pub(super) const STATIC_POLICY_OWNER: &str = "actrail.static";
const GENERATED_RULE_ID_PREFIX: &str = "network-rule";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StoredNetworkRule {
    pub(super) owner_instance_id: String,
    pub(super) rule_id: String,
    pub(super) decision: NetworkPolicyDecision,
    pub(super) remote: NetworkPolicyRemoteSelector,
    pub(super) gray_target: Option<String>,
    pub(super) timeout_ms: Option<u64>,
    pub(super) concurrency_limit: Option<u32>,
    pub(super) fallback: Option<EnforcementDecision>,
    pub(super) rule_revision: u64,
    pub(super) updated_sequence: u64,
}

impl StoredNetworkRule {
    fn from_draft(
        owner_instance_id: &str,
        item_rule_id: Option<&str>,
        draft: &NetworkPolicyRuleDraft,
        remote: NetworkPolicyRemoteSelector,
        rule_revision: u64,
        updated_sequence: u64,
        generated_rule_id: u64,
    ) -> Result<Self, String> {
        let rule_id = item_rule_id
            .map(str::to_string)
            .or_else(|| draft.rule_id.clone())
            .unwrap_or_else(|| format!("{GENERATED_RULE_ID_PREFIX}-{generated_rule_id}"));
        NetworkRuleValidator::rule_id(&rule_id)?;
        Ok(Self {
            owner_instance_id: owner_instance_id.to_string(),
            rule_id,
            decision: draft.decision,
            remote,
            gray_target: draft.gray_target.clone(),
            timeout_ms: draft.timeout_ms,
            concurrency_limit: draft.concurrency_limit,
            fallback: draft.fallback.map(verdict_decision),
            rule_revision,
            updated_sequence,
        })
    }

    fn view(&self) -> NetworkPolicyRuleView {
        NetworkPolicyRuleView {
            rule_id: self.rule_id.clone(),
            owner_instance_id: self.owner_instance_id.clone(),
            decision: self.decision,
            remote: self.remote.to_string(),
            gray_target: self.gray_target.clone(),
            timeout_ms: self.timeout_ms,
            concurrency_limit: self.concurrency_limit,
            fallback: self.fallback.map(enforcement_verdict),
            rule_revision: self.rule_revision,
            updated_sequence: self.updated_sequence,
        }
    }

    fn matches_filter(
        &self,
        decision: Option<NetworkPolicyDecision>,
        remote: Option<NetworkPolicyRemoteSelector>,
    ) -> bool {
        decision.is_none_or(|decision| decision == self.decision)
            && remote.is_none_or(|remote| remote == self.remote)
    }
}

#[derive(Clone, Debug)]
pub(super) struct NetworkPolicyStore {
    revision: u64,
    next_rule_id: u64,
    next_sequence: u64,
    by_owner: BTreeMap<String, BTreeMap<String, StoredNetworkRule>>,
    effective_by_endpoint: BTreeMap<SocketAddr, StoredNetworkRule>,
    effective_by_ip: BTreeMap<IpAddr, StoredNetworkRule>,
}

impl NetworkPolicyStore {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(format!(
                    "read network control rules {} failed: {error}",
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
                .ok_or_else(|| "network policy sequence overflow".to_string())?;
            let rule = StaticRuleParser::parse(line, store.next_sequence)
                .map_err(|message| format!("{}:{}: {message}", path.display(), index + 1))?;
            if static_rules.insert(rule.rule_id.clone(), rule).is_some() {
                return Err(format!(
                    "{}:{}: duplicate network rule id",
                    path.display(),
                    index + 1
                ));
            }
        }
        NetworkRuleValidator::owner_rules(STATIC_POLICY_OWNER, &static_rules)?;
        NetworkRuleValidator::unique_remotes(std::iter::once((
            STATIC_POLICY_OWNER,
            &static_rules,
        )))?;
        if !static_rules.is_empty() {
            store.revision = 1;
            for rule in static_rules.values_mut() {
                rule.rule_revision = store.revision;
            }
            store
                .by_owner
                .insert(STATIC_POLICY_OWNER.to_string(), static_rules);
            store.rebuild_effective()?;
        }
        Ok(store)
    }

    fn empty() -> Self {
        Self {
            revision: 0,
            next_rule_id: 1,
            next_sequence: 0,
            by_owner: BTreeMap::new(),
            effective_by_endpoint: BTreeMap::new(),
            effective_by_ip: BTreeMap::new(),
        }
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    pub(super) fn find(&self, endpoint: &SocketAddr) -> Option<&StoredNetworkRule> {
        self.effective_by_endpoint
            .get(endpoint)
            .or_else(|| self.effective_by_ip.get(&endpoint.ip()))
    }

    pub(super) fn is_rule_current(&self, rule: &StoredNetworkRule) -> bool {
        self.by_owner
            .get(&rule.owner_instance_id)
            .and_then(|rules| rules.get(&rule.rule_id))
            .is_some_and(|current| current.rule_revision == rule.rule_revision)
    }

    pub(super) fn list(
        &self,
        filter: NetworkPolicyListFilter,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<NetworkPolicyListResult, String> {
        if limit == 0 {
            return Err("network policy list limit must be positive".to_string());
        }
        let start = cursor
            .map(|raw| {
                raw.parse::<usize>()
                    .map_err(|error| format!("invalid network policy cursor {raw}: {error}"))
            })
            .transpose()?
            .unwrap_or(0);
        let limit = usize::try_from(limit)
            .map_err(|error| format!("network policy list limit overflow: {error}"))?;
        let remote = filter
            .remote
            .as_deref()
            .map(NetworkRuleValidator::selector)
            .transpose()?;
        let mut views = self
            .by_owner
            .values()
            .flat_map(|rules| rules.values())
            .filter(|rule| rule.matches_filter(filter.decision, remote))
            .map(StoredNetworkRule::view)
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
        Ok(NetworkPolicyListResult {
            rules,
            next_cursor: (next < total).then(|| next.to_string()),
            source_revision: self.revision,
        })
    }

    pub(super) fn match_dry_run(
        &self,
        request: NetworkPolicyMatchDryRunRequest,
        default_decision: NetworkPolicyDecision,
    ) -> Result<NetworkPolicyMatchDryRunResult, String> {
        let endpoint = NetworkRuleValidator::endpoint(&request.remote)?;
        let matched = self.find(&endpoint);
        Ok(NetworkPolicyMatchDryRunResult {
            matched: matched.is_some(),
            decision: matched
                .map(|rule| rule.decision)
                .unwrap_or(default_decision),
            rule_id: matched.map(|rule| rule.rule_id.clone()),
            owner_instance_id: matched.map(|rule| rule.owner_instance_id.clone()),
            resolved_remote: endpoint.to_string(),
            rule_revision: matched.map(|rule| rule.rule_revision),
            source_revision: self.revision,
        })
    }

    pub(super) fn validate_apply<F>(
        &self,
        owner_instance_id: &str,
        grants: &[NetworkPolicyRulesApplyGrant],
        request: &NetworkPolicyApplyRequest,
        target_active: F,
    ) -> NetworkPolicyApplyResult
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
        grants: &[NetworkPolicyRulesApplyGrant],
        request: NetworkPolicyApplyRequest,
        target_active: F,
    ) -> NetworkPolicyApplyResult
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
            return Err("static network policy owner cannot be removed".to_string());
        }
        if self.by_owner.remove(owner_instance_id).is_none() {
            return Ok(false);
        }
        self.bump_revision()?;
        self.rebuild_effective()?;
        Ok(true)
    }

    fn candidate_after<F>(
        &self,
        owner_instance_id: &str,
        grants: &[NetworkPolicyRulesApplyGrant],
        request: &NetworkPolicyApplyRequest,
        target_active: F,
    ) -> Result<Self, Vec<NetworkPolicyApplyError>>
    where
        F: Fn(&str) -> bool,
    {
        let mut errors = NetworkRuleValidator::request(self.revision, owner_instance_id, request);
        if !errors.is_empty() {
            return Err(errors);
        }
        let next_revision = self.revision.checked_add(1).ok_or_else(|| {
            vec![NetworkRuleValidator::apply_error(
                0,
                "revision-overflow",
                "network policy revision overflow",
            )]
        })?;
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
                errors.push(NetworkRuleValidator::apply_error(
                    index as u32,
                    "invalid-rule",
                    message,
                ));
            }
        }
        if errors.is_empty()
            && let Err(message) = NetworkRuleValidator::owner_rules(owner_instance_id, &owner_rules)
        {
            errors.push(NetworkRuleValidator::apply_error(
                0,
                "duplicate-scope",
                message,
            ));
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        if !owner_rules.is_empty() {
            candidate
                .by_owner
                .insert(owner_instance_id.to_string(), owner_rules);
        }
        if let Err(message) = NetworkRuleValidator::unique_remotes(
            candidate
                .by_owner
                .iter()
                .map(|(owner, rules)| (owner.as_str(), rules)),
        ) {
            return Err(vec![NetworkRuleValidator::apply_error(
                0,
                "endpoint-conflict",
                message,
            )]);
        }
        candidate.revision = next_revision;
        candidate.rebuild_effective().map_err(|message| {
            vec![NetworkRuleValidator::apply_error(
                0,
                "endpoint-conflict",
                message,
            )]
        })?;
        Ok(candidate)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_item<F>(
        &mut self,
        owner_instance_id: &str,
        owner_rules: &mut BTreeMap<String, StoredNetworkRule>,
        item: &NetworkPolicyPatchItem,
        next_revision: u64,
        target_active: &F,
        grants: &[NetworkPolicyRulesApplyGrant],
    ) -> Result<(), String>
    where
        F: Fn(&str) -> bool,
    {
        match item.op {
            NetworkPolicyPatchOp::Delete => {
                if item.rule.is_some() {
                    return Err("delete item must not include rule".to_string());
                }
                let rule_id = item
                    .rule_id
                    .as_deref()
                    .ok_or_else(|| "delete item requires rule_id".to_string())?;
                owner_rules
                    .remove(rule_id)
                    .ok_or_else(|| format!("network rule {rule_id} not found for owner"))?;
                Ok(())
            }
            NetworkPolicyPatchOp::Upsert => {
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
                let remote = NetworkRuleValidator::shape(draft)?;
                NetworkRuleValidator::draft_grant(grants, draft, remote)?;
                if draft.decision == NetworkPolicyDecision::Gray {
                    let target = draft
                        .gray_target
                        .as_deref()
                        .ok_or_else(|| "gray network rule requires gray_target".to_string())?;
                    if target == owner_instance_id {
                        return Err("gray network rule cannot target its policy owner".to_string());
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
                    .ok_or_else(|| "network policy sequence overflow".to_string())?;
                let generated_rule_id = self.next_rule_id;
                let rule = StoredNetworkRule::from_draft(
                    owner_instance_id,
                    item.rule_id.as_deref(),
                    draft,
                    remote,
                    next_revision,
                    self.next_sequence,
                    generated_rule_id,
                )?;
                if item.rule_id.is_none() && draft.rule_id.is_none() {
                    self.next_rule_id = self
                        .next_rule_id
                        .checked_add(1)
                        .ok_or_else(|| "network policy generated rule id overflow".to_string())?;
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
            .ok_or_else(|| "network policy revision overflow".to_string())?;
        Ok(())
    }

    fn rebuild_effective(&mut self) -> Result<(), String> {
        let mut by_ip = BTreeMap::new();
        for rule in self.by_owner.values().flat_map(|rules| rules.values()) {
            let NetworkPolicyRemoteSelector::AnyPort(ip) = rule.remote else {
                continue;
            };
            if let Some(existing) = by_ip.insert(ip, rule.clone()) {
                return Err(format!(
                    "network remote {} is owned by both {} and {}",
                    rule.remote, existing.owner_instance_id, rule.owner_instance_id
                ));
            }
        }
        let mut by_endpoint = BTreeMap::new();
        for rule in self.by_owner.values().flat_map(|rules| rules.values()) {
            let NetworkPolicyRemoteSelector::Endpoint(endpoint) = rule.remote else {
                continue;
            };
            if let Some(existing) = by_ip.get(&endpoint.ip()) {
                return Err(format!(
                    "network remote {} overlaps {} owned by {}",
                    rule.remote, existing.remote, existing.owner_instance_id
                ));
            }
            if let Some(existing) = by_endpoint.insert(endpoint, rule.clone()) {
                return Err(format!(
                    "network remote {} is owned by both {} and {}",
                    rule.remote, existing.owner_instance_id, rule.owner_instance_id
                ));
            }
        }
        self.effective_by_endpoint = by_endpoint;
        self.effective_by_ip = by_ip;
        Ok(())
    }
}

fn accepted_result(revision: u64, applied_count: u32) -> NetworkPolicyApplyResult {
    NetworkPolicyApplyResult {
        status: NetworkPolicyApplyStatus::Accepted,
        new_revision: revision,
        applied_count,
        rejected_count: 0,
        errors: Vec::new(),
    }
}

fn rejected_result(
    revision: u64,
    errors: Vec<NetworkPolicyApplyError>,
) -> NetworkPolicyApplyResult {
    NetworkPolicyApplyResult {
        status: NetworkPolicyApplyStatus::Rejected,
        new_revision: revision,
        applied_count: 0,
        rejected_count: errors.len() as u32,
        errors,
    }
}

fn verdict_decision(verdict: ControlVerdict) -> EnforcementDecision {
    match verdict {
        ControlVerdict::Allow => EnforcementDecision::Allow,
        ControlVerdict::Deny => EnforcementDecision::Deny,
    }
}

fn enforcement_verdict(decision: EnforcementDecision) -> ControlVerdict {
    match decision {
        EnforcementDecision::Allow => ControlVerdict::Allow,
        EnforcementDecision::Deny => ControlVerdict::Deny,
    }
}
