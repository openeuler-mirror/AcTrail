//! Provider-label output and confidence contracts.

use provider_evidence::EvidenceBundle;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderLabelRecord {
    pub provider: String,
    pub confidence_millis: Option<u16>,
    pub rationale: Option<String>,
}

impl ProviderLabelRecord {
    pub fn unknown(label: impl Into<String>) -> Self {
        Self {
            provider: label.into(),
            confidence_millis: None,
            rationale: None,
        }
    }
}

pub trait ProviderClassifier {
    fn classify(&self, evidence: &EvidenceBundle) -> ProviderLabelRecord;
}
