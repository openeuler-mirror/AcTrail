//! Mapping from available evidence to rule-evaluation inputs.

use provider_evidence::EvidenceBundle;

use crate::rules::ProviderRule;

pub fn matches_rule(rule: &ProviderRule, evidence: &EvidenceBundle) -> bool {
    evidence
        .get(&rule.field)
        .map(|value| value == rule.equals)
        .unwrap_or(false)
}
