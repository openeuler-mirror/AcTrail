//! Stored network rule and its public representation.
use super::validation::NetworkRuleValidator;
use config_core::daemon::EnforcementDecision;
use plugin_system::{
    ControlVerdict, NetworkPolicyDecision, NetworkPolicyEndpointSelector, NetworkPolicyOperation,
    NetworkPolicyProtocol, NetworkPolicyRuleDraft, NetworkPolicyRuleView,
};

pub(super) const STATIC_POLICY_OWNER: &str = "actrail.static";
const GENERATED_RULE_ID_PREFIX: &str = "network-rule";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredNetworkRule {
    pub owner_instance_id: String,
    pub rule_id: String,
    pub decision: NetworkPolicyDecision,
    pub operation: NetworkPolicyOperation,
    pub protocol: NetworkPolicyProtocol,
    pub endpoint: NetworkPolicyEndpointSelector,
    pub gray_target: Option<String>,
    pub timeout_ms: Option<u64>,
    pub concurrency_limit: Option<u32>,
    pub fallback: Option<EnforcementDecision>,
    pub rule_revision: u64,
    pub(super) updated_sequence: u64,
}

impl StoredNetworkRule {
    pub(super) fn from_draft(
        owner_instance_id: &str,
        item_rule_id: Option<&str>,
        draft: &NetworkPolicyRuleDraft,
        endpoint: NetworkPolicyEndpointSelector,
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
            operation: draft.operation,
            protocol: draft.protocol,
            endpoint,
            gray_target: draft.gray_target.clone(),
            timeout_ms: draft.timeout_ms,
            concurrency_limit: draft.concurrency_limit,
            fallback: draft.fallback.map(Self::verdict_decision),
            rule_revision,
            updated_sequence,
        })
    }

    pub(super) fn view(&self) -> NetworkPolicyRuleView {
        NetworkPolicyRuleView {
            rule_id: self.rule_id.clone(),
            owner_instance_id: self.owner_instance_id.clone(),
            decision: self.decision,
            operation: self.operation,
            protocol: self.protocol,
            endpoint: self.endpoint.to_string(),
            gray_target: self.gray_target.clone(),
            timeout_ms: self.timeout_ms,
            concurrency_limit: self.concurrency_limit,
            fallback: self.fallback.map(Self::enforcement_verdict),
            rule_revision: self.rule_revision,
            updated_sequence: self.updated_sequence,
        }
    }

    pub(super) fn matches_filter(
        &self,
        decision: Option<NetworkPolicyDecision>,
        endpoint: Option<NetworkPolicyEndpointSelector>,
    ) -> bool {
        decision.is_none_or(|decision| decision == self.decision)
            && endpoint.is_none_or(|endpoint| endpoint == self.endpoint)
    }
}

impl StoredNetworkRule {
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
}
