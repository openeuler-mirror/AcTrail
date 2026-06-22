use std::time::SystemTime;

use export_core::ExportRuntime;
use model_core::ids::DiagnosticId;
use storage_core::StorageBackend;

use crate::observed::{ObservedRecordBatch, ObservedRecordRecorder, ObservedRecordWriteSession};
use crate::semantic::{
    RecordingError, SemanticActionBatch, SemanticActionExportRecorder, TraceRecordLookup,
};
use crate::transaction::RecordingTransaction;

pub(crate) struct ObservedRecordCommitCoordinator<'a> {
    storage: &'a mut dyn StorageBackend,
    export_runtime: &'a ExportRuntime,
}

impl<'a> ObservedRecordCommitCoordinator<'a> {
    pub(crate) fn new(
        storage: &'a mut dyn StorageBackend,
        export_runtime: &'a ExportRuntime,
    ) -> Self {
        Self {
            storage,
            export_runtime,
        }
    }

    pub(crate) fn persist_batch_then_export(
        &mut self,
        batch: ObservedRecordBatch,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        let transaction = RecordingTransaction::begin(self.storage)?;
        let write_result = ObservedRecordRecorder::new(self.storage).persist_batch(batch);
        transaction.commit_or_rollback_then(
            write_result,
            |error| error,
            |commit| {
                SemanticActionExportRecorder::new(self.storage, self.export_runtime)
                    .publish_batches_by_trace(
                        traces,
                        commit.into_semantic_actions(),
                        emitted_at,
                        next_diagnostic_id,
                    )
            },
        )
    }

    pub(crate) fn write_session_then_export(
        &mut self,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
        write: impl FnOnce(
            &mut ObservedRecordWriteSession<'_>,
        ) -> Result<SemanticActionBatch, RecordingError>,
    ) -> Result<(), RecordingError> {
        let transaction = RecordingTransaction::begin(self.storage)?;
        let write_result = {
            let mut session = ObservedRecordWriteSession::new(self.storage);
            write(&mut session)
        };
        transaction.commit_or_rollback_then(
            write_result,
            |error| error,
            |semantic_actions| {
                SemanticActionExportRecorder::new(self.storage, self.export_runtime)
                    .publish_batches_by_trace(
                        traces,
                        semantic_actions,
                        emitted_at,
                        next_diagnostic_id,
                    )
            },
        )
    }
}
