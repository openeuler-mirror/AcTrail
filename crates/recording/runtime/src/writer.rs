use std::time::{Duration, SystemTime};

use export_core::{ExportPublishReport, ExportRuntime};
use model_core::diagnostics::DiagnosticRecord;
use model_core::event::DomainEvent;
use model_core::ids::DiagnosticId;
use model_core::process::{ProcessMembership, ProcessRecord};
use storage_core::StorageBackend;

use crate::DeliveryReport;
use crate::commit::ObservedRecordCommitCoordinator;
use crate::diagnostics::export_drop_diagnostics;
use crate::observed::{
    ObservedRecordBatch, ObservedRecordCommit, ObservedRecordRecorder, ObservedRecordWriteSession,
    TraceStateRecord,
};
use crate::semantic::{
    RecordingError, SemanticActionBatch, SemanticActionExportRecorder, SemanticActionPublication,
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
        self.persist_batch(ObservedRecordBatch::from_trace_state(
            trace_state,
            Vec::new(),
            Vec::new(),
        ))
        .map(|_| ())
    }

    pub fn persist_trace_state_with_memberships(
        &mut self,
        trace_state: TraceStateRecord,
        memberships: Vec<ProcessMembership>,
    ) -> Result<(), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_trace_state(
            trace_state,
            memberships,
            Vec::new(),
        ))
        .map(|_| ())
    }

    pub fn persist_trace_state_with_process_records(
        &mut self,
        trace_state: TraceStateRecord,
        memberships: Vec<ProcessMembership>,
        process_records: Vec<ProcessRecord>,
    ) -> Result<(), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_trace_state(
            trace_state,
            memberships,
            process_records,
        ))
        .map(|_| ())
    }

    pub fn persist_memberships(
        &mut self,
        memberships: Vec<ProcessMembership>,
    ) -> Result<(), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_memberships(memberships))
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
        memberships: Vec<ProcessMembership>,
        process_records: Vec<ProcessRecord>,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> DeliveryReport {
        let batch = ObservedRecordBatch::from_live_events(
            events,
            diagnostics,
            semantic_actions,
            trace_states,
            memberships,
            process_records,
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
        export_batch: SemanticActionBatch,
        observe_semantic_flush: impl FnOnce(Duration),
        write: impl FnOnce(&mut ObservedRecordWriteSession<'_>) -> Result<(), RecordingError>,
    ) -> DeliveryReport {
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

    pub fn export_semantic_action_batch_for_trace(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> DeliveryReport {
        let publication = SemanticActionExportRecorder::new(export_runtime)
            .publish_batch_for_trace(
                traces,
                semantic_actions.as_record_batch(),
                emitted_at,
                next_diagnostic_id,
            );
        self.persist_publication(publication)
    }

    pub fn export_semantic_action_batches(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> DeliveryReport {
        let publication = SemanticActionExportRecorder::new(export_runtime)
            .publish_batches_by_trace(traces, semantic_actions, emitted_at, next_diagnostic_id);
        self.persist_publication(publication)
    }

    pub fn export_final_semantic_action_batch_for_trace(
        &mut self,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        trace_id: model_core::ids::TraceId,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> DeliveryReport {
        let publication = SemanticActionExportRecorder::new(export_runtime)
            .publish_final_batch_for_trace(
                traces,
                trace_id,
                semantic_actions.as_record_batch(),
                emitted_at,
                next_diagnostic_id,
            );
        self.persist_publication(publication)
    }

    pub(crate) fn persist_publication(
        &mut self,
        publication: SemanticActionPublication,
    ) -> DeliveryReport {
        let mut delivery = publication.delivery;
        for diagnostic in publication.diagnostics {
            delivery.record(
                crate::DeliveryFailureKind::Storage,
                self.storage
                    .append_diagnostic(diagnostic)
                    .map_err(RecordingError::from),
            );
        }
        delivery
    }
}
