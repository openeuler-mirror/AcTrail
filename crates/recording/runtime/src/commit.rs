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
        let export_batch = batch.semantic_actions().clone();
        let persist_result = match RecordingTransaction::begin(self.storage) {
            Ok(transaction) => {
                let write_result = ObservedRecordRecorder::new(self.storage).persist_batch(batch);
                transaction
                    .commit_or_rollback(write_result, |error| error)
                    .map(|_| ())
            }
            Err(error) => Err(error),
        };
        let export_result = SemanticActionExportRecorder::new(self.storage, self.export_runtime)
            .publish_batches_by_trace(traces, export_batch, emitted_at, next_diagnostic_id);
        combine_independent_results(persist_result, export_result)
    }

    pub(crate) fn write_session_then_export(
        &mut self,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
        export_batch: SemanticActionBatch,
        write: impl FnOnce(&mut ObservedRecordWriteSession<'_>) -> Result<(), RecordingError>,
    ) -> Result<(), RecordingError> {
        let persist_result = match RecordingTransaction::begin(self.storage) {
            Ok(transaction) => {
                let write_result = {
                    let mut session = ObservedRecordWriteSession::new(self.storage);
                    write(&mut session)
                };
                transaction
                    .commit_or_rollback(write_result, |error| error)
                    .map(|_| ())
            }
            Err(error) => Err(error),
        };
        let export_result = SemanticActionExportRecorder::new(self.storage, self.export_runtime)
            .publish_batches_by_trace(traces, export_batch, emitted_at, next_diagnostic_id);
        combine_independent_results(persist_result, export_result)
    }
}

fn combine_independent_results(
    persist_result: Result<(), RecordingError>,
    export_result: Result<(), RecordingError>,
) -> Result<(), RecordingError> {
    match (persist_result, export_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(persist), Err(export)) => Err(RecordingError::new(
            "persist_and_export",
            format!(
                "persistence failed at {}: {}; export failed at {}: {}",
                persist.stage, persist.message, export.stage, export.message
            ),
        )),
    }
}
