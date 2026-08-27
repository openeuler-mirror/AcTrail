//! Provider-label projection from explicit evidence inputs.

use provider_evidence::EvidenceBundle;
use provider_label::{ProviderClassifier, ProviderLabelRecord};

use crate::config::RuleSetAdapterConfig;
use crate::evidence_map::matches_rule;
use crate::rules::ProviderRule;

pub struct RuleSetClassifier {
    config: RuleSetAdapterConfig,
    rules: Vec<ProviderRule>,
}

impl RuleSetClassifier {
    pub fn new(config: RuleSetAdapterConfig, rules: Vec<ProviderRule>) -> Self {
        Self { config, rules }
    }
}

impl ProviderClassifier for RuleSetClassifier {
    fn classify(&self, evidence: &EvidenceBundle) -> ProviderLabelRecord {
        self.rules
            .iter()
            .find(|rule| matches_rule(rule, evidence))
            .map(|rule| ProviderLabelRecord {
                provider: rule.provider.clone(),
                confidence_millis: Some(rule.confidence_millis),
                rationale: rule.rationale.clone(),
            })
            .unwrap_or_else(|| {
                ProviderLabelRecord::unknown(self.config.unknown_provider_label.clone())
            })
    }
}
