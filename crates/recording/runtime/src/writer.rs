use std::time::SystemTime;

use export_core::ExportRuntime;
use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::DiagnosticId;
use semantic_action::{SemanticAction, SemanticActionLink};
use storage_core::StorageBackend;

use crate::commit::ObservedRecordCommitCoordinator;
use crate::observed::{
    ObservedRecordBatch, ObservedRecordCommit, ObservedRecordRecorder, ObservedRecordWriteSession,
    TraceStateRecord,
};
use crate::semantic::{
    RecordingError, SemanticActionBatch, SemanticActionExportRecorder, SemanticActionRecordBatch,
    TraceRecordLookup,
};

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

    pub fn persist_trace_state(
        &mut self,
        trace_state: TraceStateRecord,
    ) -> Result<(), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_trace_state(trace_state))
            .map(|_| ())
    }

    pub fn persist_diagnostic(
        &mut self,
        diagnostic: DiagnosticRecord,
    ) -> Result<(), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_diagnostic(diagnostic))
            .map(|_| ())
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
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        let batch = ObservedRecordBatch::from_live_events(
            events,
            diagnostics,
            semantic_actions,
            trace_states,
        );
        ObservedRecordCommitCoordinator::new(self.storage, export_runtime)
            .persist_batch_then_export(batch, traces, emitted_at, next_diagnostic_id)
    }

    pub fn write_session_then_export(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
        write: impl FnOnce(
            &mut ObservedRecordWriteSession<'_>,
        ) -> Result<SemanticActionBatch, RecordingError>,
    ) -> Result<(), RecordingError> {
        ObservedRecordCommitCoordinator::new(self.storage, export_runtime)
            .write_session_then_export(traces, emitted_at, next_diagnostic_id, write)
    }

    pub fn export_semantic_actions_for_trace(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        actions: &[SemanticAction],
        links: &[SemanticActionLink],
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        SemanticActionExportRecorder::new(self.storage, export_runtime).publish_batch_for_trace(
            traces,
            SemanticActionRecordBatch::new(actions, links),
            emitted_at,
            next_diagnostic_id,
        )
    }
}
