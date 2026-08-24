//! Materialization and fail-local persistence for typed LLM pipeline diagnostics.

use model_core::diagnostics::{
    DiagnosticKind, DiagnosticRecord, DiagnosticSeverity, LlmPipelineDiagnostic,
    LlmPipelineDiagnosticSeverity,
};
use recording_runtime::SemanticActionBatch;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;

const LLM_PIPELINE_COMPONENT: &str = "llm_pipeline";

impl StorageAttachService {
    pub(in crate::services) fn persist_llm_pipeline_diagnostics_fail_local(
        &mut self,
        trace_runtime: &TraceRuntime,
        drafts: Vec<LlmPipelineDiagnostic>,
    ) {
        if drafts.is_empty() {
            return;
        }
        if let Err(error) = self.storage.append_llm_pipeline_diagnostics(&drafts) {
            tracing::warn!(
                error = ?error,
                count = drafts.len(),
                "LLM pipeline diagnostic table write failed; continuing observation"
            );
        }

        let mut diagnostics = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let diagnostic_id = match self.next_diagnostic_id() {
                Ok(id) => id,
                Err(error) => {
                    tracing::warn!(
                        error = ?error,
                        "LLM pipeline diagnostic id allocation failed; continuing observation"
                    );
                    return;
                }
            };
            let kind = if draft.code().is_runtime_drop() {
                DiagnosticKind::RuntimeDropped
            } else {
                DiagnosticKind::RuntimeFailure
            };
            let severity = match draft.severity() {
                LlmPipelineDiagnosticSeverity::Info => DiagnosticSeverity::Info,
                LlmPipelineDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
                LlmPipelineDiagnosticSeverity::Error => DiagnosticSeverity::Error,
            };
            let mut record = DiagnosticRecord::new(
                diagnostic_id,
                Some(draft.trace_id()),
                kind,
                severity,
                draft.observed_at(),
                format!(
                    "LLM pipeline diagnostic: code={} stage={}",
                    draft.code().as_u16(),
                    draft.stage().as_u8()
                ),
            )
            .with_process(*draft.process())
            .with_metadata("component", LLM_PIPELINE_COMPONENT)
            .with_metadata("code", draft.code().as_u16().to_string())
            .with_metadata("stage", draft.stage().as_u8().to_string());
            if let Some(stream_key) = draft.stream_key() {
                record = record.with_metadata("stream_key", stream_key);
            }
            if let Some(discarded_bytes) = draft.discarded_bytes() {
                record = record.with_metadata("discarded_bytes", discarded_bytes.to_string());
            }
            if let Some(discarded_entries) = draft.discarded_entries() {
                record = record.with_metadata("discarded_entries", discarded_entries.to_string());
            }
            diagnostics.push(record);
        }

        if let Err(error) = self.persist_observed_batch_then_publish(
            trace_runtime,
            Vec::new(),
            diagnostics,
            SemanticActionBatch::default(),
            Vec::new(),
            Vec::new(),
        ) {
            tracing::warn!(
                error = ?error,
                "LLM pipeline diagnostic export failed; continuing observation"
            );
        }
    }
}
