use std::time::SystemTime;

use export_core::ExportPublishReport;
use model_core::diagnostics::DiagnosticRecord;
use model_core::ids::DiagnosticId;
use model_core::process::{ProcessMembership, ProcessRecord};
use storage_core::StorageBackend;

use crate::DeliveryReport;
use crate::diagnostics::export_drop_diagnostics;
use crate::observed::{
    ObservedRecordBatch, ObservedRecordCommit, ObservedRecordRecorder, TraceStateRecord,
};
use crate::semantic::{RecordingError, SemanticActionBatch, SemanticActionPublication};

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
    ) -> Result<(SemanticActionBatch, DeliveryReport), RecordingError> {
        self.persist_batch(ObservedRecordBatch::from_semantic_actions(semantic_actions))
            .map(ObservedRecordCommit::into_semantic_delivery)
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
