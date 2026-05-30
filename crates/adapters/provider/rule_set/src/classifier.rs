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

#[cfg(test)]
mod tests {
    use provider_evidence::EvidenceBundle;
    use provider_label::ProviderClassifier;

    use crate::config::RuleSetAdapterConfig;
    use crate::rules::ProviderRule;

    use super::RuleSetClassifier;

    #[test]
    fn matching_rule_produces_provider_label() {
        let classifier = RuleSetClassifier::new(
            RuleSetAdapterConfig {
                unknown_provider_label: "unknown".to_string(),
            },
            vec![ProviderRule {
                field: "remote".to_string(),
                equals: "api.openai.com".to_string(),
                provider: "openai".to_string(),
                confidence_millis: 900,
                rationale: Some("explicit hostname match".to_string()),
            }],
        );
        let mut evidence = EvidenceBundle::new();
        evidence.insert("remote", "api.openai.com");

        let label = classifier.classify(&evidence);
        assert_eq!(label.provider, "openai");
        assert_eq!(label.confidence_millis, Some(900));
    }

    #[test]
    fn unknown_rule_keeps_unknown_label() {
        let classifier = RuleSetClassifier::new(
            RuleSetAdapterConfig {
                unknown_provider_label: "unknown".to_string(),
            },
            Vec::new(),
        );

        let label = classifier.classify(&EvidenceBundle::new());
        assert_eq!(label.provider, "unknown");
        assert_eq!(label.confidence_millis, None);
    }
}
