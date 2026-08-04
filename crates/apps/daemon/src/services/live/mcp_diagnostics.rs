use control_contract::reply::ControlError;
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use recording_runtime::SemanticActionBatch;
use semantic_action_runtime::live::LiveMcpStdioDiagnostic;
use trace_runtime::registry::TraceRuntime;

use crate::services::attach::StorageAttachService;

const MCP_STDIO_COMPONENT: &str = "mcp_stdio";

impl StorageAttachService {
    pub(super) fn materialize_mcp_stdio_diagnostics(
        &mut self,
        drafts: Vec<LiveMcpStdioDiagnostic>,
    ) -> Result<Vec<DiagnosticRecord>, ControlError> {
        drafts
            .into_iter()
            .map(|draft| {
                let mut record = DiagnosticRecord::new(
                    self.next_diagnostic_id()?,
                    Some(draft.trace_id()),
                    DiagnosticKind::RuntimeDropped,
                    DiagnosticSeverity::Warning,
                    draft.emitted_at(),
                    draft.message(),
                )
                .with_process(draft.process().clone())
                .with_metadata("code", draft.code())
                .with_metadata("component", MCP_STDIO_COMPONENT)
                .with_metadata("stage", draft.stage())
                .with_metadata("reason", draft.reason())
                .with_metadata("recoverable", draft.recoverable().to_string());
                if let Some(stream) = draft.stream() {
                    record = record.with_metadata("stream", stream);
                }
                Ok(record)
            })
            .collect()
    }

    pub(in crate::services) fn persist_mcp_stdio_diagnostics_impl(
        &mut self,
        trace_runtime: &TraceRuntime,
        drafts: Vec<LiveMcpStdioDiagnostic>,
    ) -> Result<(), ControlError> {
        if drafts.is_empty() {
            return Ok(());
        }
        let diagnostics = self.materialize_mcp_stdio_diagnostics(drafts)?;
        self.persist_observed_batch_then_publish(
            trace_runtime,
            Vec::new(),
            diagnostics,
            SemanticActionBatch::default(),
            Vec::new(),
            Vec::new(),
        )
    }
}
