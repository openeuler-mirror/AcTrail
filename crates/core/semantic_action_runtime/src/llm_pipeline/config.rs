//! Runtime-owned LLM pipeline configuration.

use config_core::daemon::SemanticRetentionConfig;

#[derive(Clone, Copy)]
pub(crate) struct StreamClassifierConfig {
    soft_sniff_max_bytes: usize,
}

impl StreamClassifierConfig {
    pub(crate) fn from_semantic_retention(config: &SemanticRetentionConfig) -> Self {
        Self {
            soft_sniff_max_bytes: usize::try_from(
                config.l0_llm_call.stream_classifier.soft_sniff_max_bytes,
            )
            .expect("validated LLM stream classifier byte budget must fit usize"),
        }
    }

    pub(crate) fn can_sniff_through(self, decoded_body_offset: usize) -> bool {
        decoded_body_offset <= self.soft_sniff_max_bytes
    }

    pub(crate) fn soft_sniff_max_bytes(self) -> usize {
        self.soft_sniff_max_bytes
    }
}
