use std::time::SystemTime;

use export_core::{ExportRuntime, SemanticActionExportBatch};
use model_core::ids::{DiagnosticId, TraceId};
use model_core::trace::TraceRecord;
use storage_core::StorageBackend;

use crate::diagnostics::export_drop_diagnostics;

use super::{RecordingError, SemanticActionRecordBatch};

const LIVE_EXPORT_STAGE: &str = "live_export";

pub trait TraceRecordLookup {
    fn trace_record(&self, trace_id: TraceId) -> Option<&TraceRecord>;
}

pub(crate) struct SemanticActionExportRecorder<'a> {
    storage: &'a mut dyn StorageBackend,
    export_runtime: &'a ExportRuntime,
}

impl<'a> SemanticActionExportRecorder<'a> {
    pub(crate) fn new(
        storage: &'a mut dyn StorageBackend,
        export_runtime: &'a ExportRuntime,
    ) -> Self {
        Self {
            storage,
            export_runtime,
        }
    }

    pub(crate) fn publish_batch(
        &mut self,
        trace: &TraceRecord,
        batch: SemanticActionRecordBatch<'_>,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        if batch.actions().is_empty() {
            return Ok(());
        }
        let report = self
            .export_runtime
            .publish_semantic_actions(SemanticActionExportBatch {
                trace,
                actions: batch.actions(),
                links: batch.links(),
            })?;
        // Export backpressure is recorded after publish so collection can continue visibly.
        let diagnostics = export_drop_diagnostics(report, emitted_at, next_diagnostic_id)?;
        for diagnostic in diagnostics {
            self.storage.append_diagnostic(diagnostic)?;
        }
        Ok(())
    }

    pub(crate) fn publish_batch_for_trace(
        &mut self,
        traces: &dyn TraceRecordLookup,
        batch: SemanticActionRecordBatch<'_>,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        if batch.actions().is_empty() {
            return Ok(());
        }
        let trace_id = batch
            .trace_id()?
            .ok_or_else(|| RecordingError::new(LIVE_EXPORT_STAGE, "empty semantic action batch"))?;
        let trace = traces
            .trace_record(trace_id)
            .ok_or_else(|| RecordingError::new(LIVE_EXPORT_STAGE, "trace not found"))?;
        self.publish_batch(trace, batch, emitted_at, next_diagnostic_id)
    }
}
