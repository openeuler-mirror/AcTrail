//! Bounded LLM-versus-ordinary-SSE classification.

use crate::llm_pipeline::config::StreamClassifierConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamClassification {
    Undetermined,
    ConfirmedLlm,
}

/// Owns the soft initial sniff window without turning budget exhaustion into
/// an ordinary-SSE verdict. Complete events arriving later remain eligible for
/// provider recognition and can trigger a one-time replay of retained bytes.
pub(in crate::llm_pipeline) struct StreamClassifier {
    config: StreamClassifierConfig,
    classification: StreamClassification,
    soft_budget_exhaustion_reported: bool,
}

impl StreamClassifier {
    pub(in crate::llm_pipeline) fn new(config: StreamClassifierConfig) -> Self {
        Self {
            config,
            classification: StreamClassification::Undetermined,
            soft_budget_exhaustion_reported: false,
        }
    }

    pub(in crate::llm_pipeline) fn belongs_to_initial_window(
        &mut self,
        event_end_offset: usize,
    ) -> bool {
        let belongs = self.config.can_sniff_through(event_end_offset);
        if !belongs && !self.soft_budget_exhaustion_reported {
            self.soft_budget_exhaustion_reported = true;
            tracing::debug!(
                event_end_offset,
                soft_sniff_max_bytes = self.config.soft_sniff_max_bytes(),
                "LLM SSE classifier exceeded its soft initial sniff window; late provider recognition remains enabled"
            );
        }
        belongs
    }

    pub(in crate::llm_pipeline) fn confirm_llm(&mut self) {
        self.classification = StreamClassification::ConfirmedLlm;
    }

    pub(in crate::llm_pipeline) fn is_confirmed_llm(&self) -> bool {
        self.classification == StreamClassification::ConfirmedLlm
    }
}
