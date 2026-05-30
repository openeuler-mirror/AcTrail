//! Adapter-local loading and shaping of provider rule configuration.

use config_core::provider_rules::ProviderRuleSetConfig;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleSetAdapterConfig {
    pub unknown_provider_label: String,
}

impl From<&ProviderRuleSetConfig> for RuleSetAdapterConfig {
    fn from(value: &ProviderRuleSetConfig) -> Self {
        Self {
            unknown_provider_label: value.unknown_provider_label.clone(),
        }
    }
}
