//! Provider-classification rule-set configuration.

use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderRuleSetConfig {
    pub rules_path: PathBuf,
    pub unknown_provider_label: String,
}
