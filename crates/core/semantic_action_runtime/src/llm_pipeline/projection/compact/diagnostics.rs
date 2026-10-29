use std::time::SystemTime;

use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use semantic_action::SemanticAction;

use super::observation::CompactLimitExceeded;

pub(super) struct CompactDiagnostics;

impl CompactDiagnostics {
    pub(super) fn entry_diagnostic(
        action: &SemanticAction,
        observed_at: SystemTime,
        code: LlmPipelineDiagnosticCode,
        entries: usize,
    ) -> LlmPipelineDiagnostic {
        LlmPipelineDiagnostic::new(
            action.trace_id,
            &action.process,
            observed_at,
            code,
            LlmPipelineDiagnosticSeverity::Warning,
            LlmPipelineDiagnosticStage::Correlation,
        )
        .with_discarded_entries(u64::try_from(entries).unwrap_or(u64::MAX))
    }

    pub(super) fn byte_diagnostic(
        action: &SemanticAction,
        observed_at: SystemTime,
        bytes: usize,
    ) -> LlmPipelineDiagnostic {
        Self::limit_diagnostic(action, observed_at, CompactLimitExceeded::Bytes(bytes))
    }

    pub(super) fn limit_diagnostic(
        action: &SemanticAction,
        observed_at: SystemTime,
        limit: CompactLimitExceeded,
    ) -> LlmPipelineDiagnostic {
        let diagnostic = LlmPipelineDiagnostic::new(
            action.trace_id,
            &action.process,
            observed_at,
            LlmPipelineDiagnosticCode::ContextCompactArtifactLimitExceeded,
            LlmPipelineDiagnosticSeverity::Warning,
            LlmPipelineDiagnosticStage::Correlation,
        );
        match limit {
            CompactLimitExceeded::Entries(entries) => {
                diagnostic.with_discarded_entries(u64::try_from(entries).unwrap_or(u64::MAX))
            }
            CompactLimitExceeded::Bytes(bytes) => {
                diagnostic.with_discarded_bytes(u64::try_from(bytes).unwrap_or(u64::MAX))
            }
            CompactLimitExceeded::BytesUnknown => diagnostic,
        }
    }
}
