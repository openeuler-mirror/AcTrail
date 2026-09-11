use std::time::{Duration, SystemTime};

use export_core::{ExportPublishReport, ExportRuntime};
use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::DiagnosticId;
use model_core::process::ProcessRecord;
use storage_core::StorageBackend;

use crate::commit::ObservedRecordCommitCoordinator;
use crate::diagnostics::export_drop_diagnostics;
use crate::observed::{
    ObservedRecordBatch, ObservedRecordCommit, ObservedRecordRecorder, ObservedRecordWriteSession,
    TraceStateRecord,
};
use crate::semantic::{
    RecordingError, SemanticActionBatch, SemanticActionExportRecorder, TraceRecordLookup,
};
use crate::transaction::RecordingTransaction;

pub struct RecordingWriter<'a> {
    storage: &'a mut dyn StorageBackend,
}

impl<'a> RecordingWriter<'a> {
    pub fn new(storage: &'a mut dyn StorageBackend) -> Self {
        Self { storage }
    }

    fn persist_batch(
        &mut self,
        batch: ObservedRecordBatch,
    ) -> Result<ObservedRecordCommit, RecordingError> {
        ObservedRecordRecorder::new(self.storage).persist_batch(batch)
    }

    fn persist_batch_with_additional_write(
        &mut self,
        batch: ObservedRecordBatch,
        additional_write: impl FnOnce(&mut dyn StorageBackend) -> Result<(), RecordingError>,
    ) -> Result<(), RecordingError> {
        let transaction = RecordingTransaction::begin(self.storage)?;
        let result = ObservedRecordRecorder::new(self.storage)
            .persist_batch(batch)
            .map(|_| ())
            .and_then(|()| additional_write(self.storage));
        transaction.commit_or_rollback(result.map(|_| ()), |error| error)
    }

    pub fn persist_trace_state(
        &mut self,
        trace_state: TraceStateRecord,
    ) -> Result<(), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_trace_state(
            trace_state,
            Vec::new(),
        ))
        .map(|_| ())
    }

    pub fn persist_trace_state_with_process_records(
        &mut self,
        trace_state: TraceStateRecord,
        process_records: Vec<ProcessRecord>,
    ) -> Result<(), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_trace_state(
            trace_state,
            process_records,
        ))
        .map(|_| ())
    }

    pub fn persist_diagnostic(
        &mut self,
        diagnostic: DiagnosticRecord,
    ) -> Result<(), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_diagnostic(diagnostic))
            .map(|_| ())
    }

    pub fn persist_export_drop_report(
        &mut self,
        report: ExportPublishReport,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        let diagnostics = export_drop_diagnostics(report, emitted_at, next_diagnostic_id)?;
        for diagnostic in diagnostics {
            self.storage.append_diagnostic(diagnostic)?;
        }
        Ok(())
    }

    pub fn persist_semantic_actions(
        &mut self,
        semantic_actions: SemanticActionBatch,
    ) -> Result<SemanticActionBatch, RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_semantic_actions(semantic_actions))
            .map(ObservedRecordCommit::into_semantic_actions)
    }

    pub fn persist_live_events_then_export(
        &mut self,
        export_runtime: &ExportRuntime,
        events: Vec<DomainEvent>,
        diagnostics: Vec<DiagnosticRecord>,
        semantic_actions: SemanticActionBatch,
        trace_states: Vec<TraceStateRecord>,
        process_records: Vec<ProcessRecord>,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        let batch = ObservedRecordBatch::from_live_events(
            events,
            diagnostics,
            semantic_actions,
            trace_states,
            process_records,
        );
        ObservedRecordCommitCoordinator::new(self.storage, export_runtime)
            .persist_batch_then_export(batch, traces, emitted_at, next_diagnostic_id)
    }

    pub fn persist_semantic_actions_with_additional_write(
        &mut self,
        semantic_actions: SemanticActionBatch,
        additional_write: impl FnOnce(&mut dyn StorageBackend) -> Result<(), RecordingError>,
    ) -> Result<(), RecordingError> {
        self.persist_batch_with_additional_write(
            ObservedRecordBatch::from_semantic_actions(semantic_actions),
            additional_write,
        )
    }

    fn finish_then_export(
        &mut self,
        persist_result: Result<(), RecordingError>,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        export_batch: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> (Result<(), RecordingError>, bool) {
        let persisted = persist_result.is_ok();
        let result = persist_result.and_then(|()| {
            SemanticActionExportRecorder::new(self.storage, export_runtime)
                .publish_batches_by_trace(traces, export_batch, emitted_at, next_diagnostic_id)
        });
        (result, persisted)
    }

    /// Persists a live batch and an additional write atomically, then exports the actions.
    pub fn persist_live_events_then_export_with_additional_write(
        &mut self,
        export_runtime: &ExportRuntime,
        events: Vec<DomainEvent>,
        diagnostics: Vec<DiagnosticRecord>,
        semantic_actions: SemanticActionBatch,
        trace_states: Vec<TraceStateRecord>,
        process_records: Vec<ProcessRecord>,
        additional_write: impl FnOnce(&mut dyn StorageBackend) -> Result<(), RecordingError>,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> (Result<(), RecordingError>, bool) {
        let export_batch = semantic_actions.clone();
        let persist_result = self.persist_batch_with_additional_write(
            ObservedRecordBatch::from_live_events(
                events,
                diagnostics,
                semantic_actions,
                trace_states,
                process_records,
            ),
            additional_write,
        );
        self.finish_then_export(
            persist_result,
            export_runtime,
            traces,
            export_batch,
            emitted_at,
            next_diagnostic_id,
        )
    }

    fn write_session_with_additional_write(
        &mut self,
        observe_semantic_flush: impl FnOnce(Duration),
        additional_write: impl FnOnce(&mut dyn StorageBackend) -> Result<(), RecordingError>,
        write: impl FnOnce(&mut ObservedRecordWriteSession<'_>) -> Result<(), RecordingError>,
    ) -> Result<(), RecordingError> {
        let transaction = RecordingTransaction::begin(self.storage)?;
        let (write_result, flush_elapsed) = {
            let mut session = ObservedRecordWriteSession::new(self.storage);
            match write(&mut session) {
                Ok(()) => {
                    let started = std::time::Instant::now();
                    (session.finish(), Some(started.elapsed()))
                }
                Err(error) => (Err(error), None),
            }
        };
        let write_result = write_result.and_then(|()| additional_write(self.storage));
        let result = transaction.commit_or_rollback(write_result, |error| error);
        if let Some(elapsed) = flush_elapsed {
            observe_semantic_flush(elapsed);
        }
        result
    }

    pub fn write_session_then_export(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
        export_batch: SemanticActionBatch,
        observe_semantic_flush: impl FnOnce(Duration),
        write: impl FnOnce(&mut ObservedRecordWriteSession<'_>) -> Result<(), RecordingError>,
    ) -> Result<(), RecordingError> {
        ObservedRecordCommitCoordinator::new(self.storage, export_runtime)
            .write_session_then_export(
                traces,
                emitted_at,
                next_diagnostic_id,
                export_batch,
                observe_semantic_flush,
                write,
            )
    }

    /// Writes a session and an additional operation atomically, then exports the actions.
    pub fn write_session_then_export_with_additional_write(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
        export_batch: SemanticActionBatch,
        observe_semantic_flush: impl FnOnce(Duration),
        additional_write: impl FnOnce(&mut dyn StorageBackend) -> Result<(), RecordingError>,
        write: impl FnOnce(&mut ObservedRecordWriteSession<'_>) -> Result<(), RecordingError>,
    ) -> (Result<(), RecordingError>, bool) {
        let persist_result = self.write_session_with_additional_write(
            observe_semantic_flush,
            additional_write,
            write,
        );
        self.finish_then_export(
            persist_result,
            export_runtime,
            traces,
            export_batch,
            emitted_at,
            next_diagnostic_id,
        )
    }

    pub fn export_semantic_action_batch_for_trace(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        SemanticActionExportRecorder::new(self.storage, export_runtime).publish_batch_for_trace(
            traces,
            semantic_actions.as_record_batch(),
            emitted_at,
            next_diagnostic_id,
        )
    }

    pub fn export_final_semantic_action_batch_for_trace(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        trace_id: model_core::ids::TraceId,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        SemanticActionExportRecorder::new(self.storage, export_runtime)
            .publish_final_batch_for_trace(
                traces,
                trace_id,
                semantic_actions.as_record_batch(),
                emitted_at,
                next_diagnostic_id,
            )
    }
}
