//! Network rule shape validation and static policy parsing.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;

use config_core::daemon::EnforcementDecision;
use plugin_system::{
    NetworkPolicyApplyError, NetworkPolicyApplyRequest, NetworkPolicyDecision,
    NetworkPolicyEndpointSelector, NetworkPolicyOperation, NetworkPolicyProtocol,
    NetworkPolicyRuleDraft, NetworkPolicyRulesApplyGrant,
};

use super::rule::{STATIC_POLICY_OWNER, StoredNetworkRule};

pub(super) struct NetworkRuleValidator;

impl NetworkRuleValidator {
    pub(super) fn rule_id(rule_id: &str) -> Result<(), String> {
        if rule_id.trim().is_empty() || rule_id.chars().any(char::is_whitespace) {
            return Err("network rule id must be non-empty and contain no whitespace".to_string());
        }
        Ok(())
    }

    pub(super) fn endpoint(endpoint: &str) -> Result<SocketAddr, String> {
        endpoint
            .parse::<SocketAddr>()
            .map_err(|error| format!("network endpoint must be a numeric IP endpoint: {error}"))
    }

    pub(super) fn selector(endpoint: &str) -> Result<NetworkPolicyEndpointSelector, String> {
        NetworkPolicyEndpointSelector::parse(endpoint)
    }

    pub(super) fn shape(
        draft: &NetworkPolicyRuleDraft,
    ) -> Result<NetworkPolicyEndpointSelector, String> {
        if draft.decision == NetworkPolicyDecision::Default {
            return Err("network rule decision cannot be default".to_string());
        }
        let has_gray_fields = draft.gray_target.is_some()
            || draft.timeout_ms.is_some()
            || draft.concurrency_limit.is_some()
            || draft.fallback.is_some();
        if draft.decision != NetworkPolicyDecision::Gray && has_gray_fields {
            return Err("allow and deny network rules cannot include gray settings".to_string());
        }
        if draft.decision == NetworkPolicyDecision::Gray {
            if draft
                .gray_target
                .as_deref()
                .is_none_or(|target| target.trim().is_empty())
            {
                return Err("gray network rule requires gray_target".to_string());
            }
            if draft.timeout_ms.is_none_or(|value| value == 0) {
                return Err("gray network rule timeout_ms must be positive".to_string());
            }
            if draft.concurrency_limit.is_none_or(|value| value == 0) {
                return Err("gray network rule concurrency_limit must be positive".to_string());
            }
            if draft.fallback.is_none() {
                return Err("gray network rule requires fallback".to_string());
            }
        }
        Self::selector(&draft.endpoint)
    }

    pub(super) fn request(
        revision: u64,
        owner_instance_id: &str,
        request: &NetworkPolicyApplyRequest,
    ) -> Vec<NetworkPolicyApplyError> {
        let mut errors = Vec::new();
        if owner_instance_id.trim().is_empty() || owner_instance_id == STATIC_POLICY_OWNER {
            errors.push(Self::apply_error(
                0,
                "invalid-owner",
                "network policy owner must be a non-static plugin instance id",
            ));
        }
        if request.base_revision != revision {
            errors.push(Self::apply_error(
                0,
                "revision-conflict",
                format!(
                    "network policy base revision {} does not match current revision {revision}",
                    request.base_revision
                ),
            ));
        }
        if request.mutation_id.trim().is_empty() {
            errors.push(Self::apply_error(
                0,
                "invalid-mutation-id",
                "network policy mutation_id must not be empty",
            ));
        }
        if request.items.is_empty() {
            errors.push(Self::apply_error(
                0,
                "empty-mutation",
                "network policy apply requires at least one item",
            ));
        }
        errors
    }

    pub(super) fn owner_rules(
        owner_instance_id: &str,
        rules: &BTreeMap<String, StoredNetworkRule>,
    ) -> Result<(), String> {
        let mut remotes = NetworkRemoteOwners::default();
        for (key, rule) in rules {
            if key != &rule.rule_id {
                return Err(format!(
                    "network policy owner {owner_instance_id} has inconsistent rule id {key}"
                ));
            }
            remotes.insert(rule, owner_instance_id)?;
        }
        Ok(())
    }

    pub(super) fn unique_remotes<'a>(
        owners: impl Iterator<Item = (&'a str, &'a BTreeMap<String, StoredNetworkRule>)>,
    ) -> Result<(), String> {
        let mut remotes = NetworkRemoteOwners::default();
        for (owner, rules) in owners {
            for rule in rules.values() {
                remotes.insert(rule, owner)?;
            }
        }
        Ok(())
    }

    pub(super) fn draft_grant(
        grants: &[NetworkPolicyRulesApplyGrant],
        draft: &NetworkPolicyRuleDraft,
        selector: NetworkPolicyEndpointSelector,
    ) -> Result<(), String> {
        if grants.iter().any(|grant| {
            grant.decision == draft.decision
                && grant.operation == draft.operation
                && grant.protocol.covers(draft.protocol)
                && grant.endpoint_scope.covers(selector)
        }) {
            return Ok(());
        }
        Err(format!(
            "missing network-policy.rules.apply grant for {} {selector}",
            draft.decision.as_str()
        ))
    }

    pub(super) fn apply_error(
        item_index: u32,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> NetworkPolicyApplyError {
        NetworkPolicyApplyError {
            item_index,
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Publication-only conflict checks. Same-owner groups need no overlap scan.
#[derive(Default)]
struct NetworkRemoteOwners<'a> {
    unique: BTreeSet<(
        NetworkPolicyOperation,
        NetworkPolicyProtocol,
        NetworkPolicyEndpointSelector,
    )>,
    owners: BTreeMap<
        &'a str,
        BTreeMap<
            NetworkPolicyOperation,
            Vec<(NetworkPolicyProtocol, NetworkPolicyEndpointSelector)>,
        >,
    >,
}

impl<'a> NetworkRemoteOwners<'a> {
    fn insert(&mut self, rule: &StoredNetworkRule, owner: &'a str) -> Result<(), String> {
        if !self
            .unique
            .insert((rule.operation, rule.protocol, rule.endpoint))
        {
            return Err(format!(
                "network endpoint {} is already owned",
                rule.endpoint
            ));
        }
        for (existing, operations) in &self.owners {
            if *existing == owner {
                continue;
            }
            if let Some(scopes) = operations.get(&rule.operation) {
                if scopes.iter().any(|(protocol, scope)| {
                    (protocol.covers(rule.protocol) || rule.protocol.covers(*protocol))
                        && scope.overlaps(rule.endpoint)
                }) {
                    return Err(format!(
                        "network endpoint {} overlaps scope owned by {existing}",
                        rule.endpoint
                    ));
                }
            }
        }
        self.owners
            .entry(owner)
            .or_default()
            .entry(rule.operation)
            .or_default()
            .push((rule.protocol, rule.endpoint));
        Ok(())
    }
}

pub(super) struct StaticRuleParser;

impl StaticRuleParser {
    pub(super) fn parse(line: &str, sequence: u64) -> Result<StoredNetworkRule, String> {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        match parts.as_slice() {
            [rule_id, decision @ ("allow" | "deny"), operation, endpoint] | [rule_id, decision @ ("allow" | "deny"), operation, endpoint, "protocol", _] => {
                let protocol = NetworkPolicyProtocol::from_wire(parts.get(5).copied().unwrap_or("any"))?;
                let operation = NetworkPolicyOperation::from_wire(operation)?;
                NetworkRuleValidator::rule_id(rule_id)?;
                Ok(StoredNetworkRule {
                    owner_instance_id: STATIC_POLICY_OWNER.to_string(),
                    rule_id: (*rule_id).to_string(),
                    decision: NetworkPolicyDecision::from_wire(decision)?,
                    operation, protocol,
                    endpoint: NetworkRuleValidator::selector(endpoint)?,
                    gray_target: None,
                    timeout_ms: None,
                    concurrency_limit: None,
                    fallback: None,
                    rule_revision: 0,
                    updated_sequence: sequence,
                })
            }
            [
                rule_id,
                "sync-plugin",
                instance,
                "timeout-ms",
                timeout_ms,
                "concurrency",
                concurrency_limit,
                "fallback",
                fallback,
                "connect",
                endpoint,
            ] => {
                NetworkRuleValidator::rule_id(rule_id)?;
                Ok(StoredNetworkRule {
                    owner_instance_id: STATIC_POLICY_OWNER.to_string(),
                    rule_id: (*rule_id).to_string(),
                    decision: NetworkPolicyDecision::Gray,
                    operation: NetworkPolicyOperation::Connect, protocol: NetworkPolicyProtocol::Any,
                    endpoint: NetworkRuleValidator::selector(endpoint)?,
                    gray_target: Some((*instance).to_string()),
                    timeout_ms: Some(Self::positive_u64("timeout-ms", timeout_ms)?),
                    concurrency_limit: Some(Self::positive_u32(
                        "concurrency",
                        concurrency_limit,
                    )?),
                    fallback: Some(Self::enforcement(fallback)?),
                    rule_revision: 0,
                    updated_sequence: sequence,
                })
            }
            _ => Err("expected: <rule-id> <allow|deny> <connect|send|bind|listen> <ip-or-cidr:port-or-range-or-*> [protocol <any|tcp|udp>], or <rule-id> sync-plugin <instance> timeout-ms <positive-ms> concurrency <positive-limit> fallback <allow|deny> connect <ip-or-cidr:port-or-range-or-*>".to_string()),
        }
    }

    fn enforcement(value: &str) -> Result<EnforcementDecision, String> {
        match value {
            "allow" => Ok(EnforcementDecision::Allow),
            "deny" => Ok(EnforcementDecision::Deny),
            _ => Err("fallback must be allow or deny".to_string()),
        }
    }

    fn positive_u64(label: &str, value: &str) -> Result<u64, String> {
        let parsed = value
            .parse::<u64>()
            .map_err(|error| format!("{label} must be a positive integer: {error}"))?;
        if parsed == 0 {
            return Err(format!("{label} must be greater than zero"));
        }
        Ok(parsed)
    }

    fn positive_u32(label: &str, value: &str) -> Result<u32, String> {
        let parsed = value
            .parse::<u32>()
            .map_err(|error| format!("{label} must be a positive integer: {error}"))?;
        if parsed == 0 {
            return Err(format!("{label} must be greater than zero"));
        }
        Ok(parsed)
    }
}
