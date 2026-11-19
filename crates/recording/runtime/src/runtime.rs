use std::time::{Duration, SystemTime};

use export_core::ExportRuntime;
use model_core::{
    diagnostics::DiagnosticRecord,
    event::DomainEvent,
    ids::{DiagnosticId, TraceId},
    process::{ProcessMembership, ProcessRecord},
};
use storage_core::StorageBackend;

use crate::commit::ObservedRecordCommitCoordinator;
use crate::observed::{ObservedRecordBatch, ObservedRecordWriteSession, TraceStateRecord};
use crate::semantic::{
    CommandDelivery, CommandDeliveryLimits, SemanticActionBatch, SemanticActionExportRecorder,
    TraceRecordLookup,
};
use crate::{DeliveryReport, RecordingError, RecordingWriter};

/// Owns deferred recording deliveries for the lifetime of the live trace service.
pub struct RecordingRuntime {
    command_delivery: CommandDelivery,
}

impl RecordingRuntime {
    pub fn new(limits: CommandDeliveryLimits) -> Self {
        Self {
            command_delivery: CommandDelivery::new(limits),
        }
    }

    pub fn forget_trace(&mut self, trace_id: TraceId) {
        self.command_delivery.forget_trace(trace_id);
    }

    pub fn clear_deliveries(&mut self) {
        self.command_delivery.clear();
    }

    pub fn persist_live_events_then_export(
        &mut self,
        storage: &mut dyn StorageBackend,
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
        ObservedRecordCommitCoordinator::new(storage, export_runtime, self)
            .persist_batch_then_export(batch, traces, emitted_at, next_diagnostic_id)
    }

    pub fn write_session_then_export(
        &mut self,
        storage: &mut dyn StorageBackend,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
        export_batch: SemanticActionBatch,
        observe_semantic_flush: impl FnOnce(Duration),
        write: impl FnOnce(&mut ObservedRecordWriteSession<'_>) -> Result<(), RecordingError>,
    ) -> DeliveryReport {
        ObservedRecordCommitCoordinator::new(storage, export_runtime, self)
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
        storage: &mut dyn StorageBackend,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> DeliveryReport {
        let publication =
            SemanticActionExportRecorder::new(export_runtime, &mut self.command_delivery)
                .publish_batch_for_trace(
                    traces,
                    semantic_actions.as_record_batch(),
                    emitted_at,
                    next_diagnostic_id,
                );
        RecordingWriter::new(storage).persist_publication(publication)
    }

    pub fn export_semantic_action_batches(
        &mut self,
        storage: &mut dyn StorageBackend,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> DeliveryReport {
        let publication =
            SemanticActionExportRecorder::new(export_runtime, &mut self.command_delivery)
                .publish_batches_by_trace(traces, semantic_actions, emitted_at, next_diagnostic_id);
        RecordingWriter::new(storage).persist_publication(publication)
    }

    pub fn export_final_semantic_action_batch_for_trace(
        &mut self,
        storage: &mut dyn StorageBackend,
        export_runtime: &ExportRuntime,
        traces: &dyn TraceRecordLookup,
        trace_id: model_core::ids::TraceId,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> DeliveryReport {
        let publication =
            SemanticActionExportRecorder::new(export_runtime, &mut self.command_delivery)
                .publish_final_batch_for_trace(
                    traces,
                    trace_id,
                    semantic_actions.as_record_batch(),
                    emitted_at,
                    next_diagnostic_id,
                );
        self.command_delivery.forget_trace(trace_id);
        RecordingWriter::new(storage).persist_publication(publication)
    }
}
