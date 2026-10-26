use std::time::{Duration, Instant, SystemTime};

use export_core::ExportRuntime;
use model_core::ids::DiagnosticId;
use storage_core::StorageBackend;

use crate::observed::{ObservedRecordBatch, ObservedRecordRecorder, ObservedRecordWriteSession};
use crate::semantic::{RecordingError, SemanticActionBatch, TraceRecordLookup};
use crate::transaction::RecordingTransaction;
use crate::{DeliveryFailureKind, DeliveryReport};

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
    ) -> DeliveryReport {
        if !self.storage.retains_observations() {
            return self.export_without_persistence(
                batch.into_semantic_actions(),
                traces,
                emitted_at,
                next_diagnostic_id,
            );
        }
        let export_batch = self
            .export_runtime
            .has_semantic_consumers()
            .then(|| batch.semantic_actions().clone());
        let persist_result = match RecordingTransaction::begin(self.storage) {
            Ok(transaction) => {
                let write_result = ObservedRecordRecorder::new(self.storage).persist_batch(batch);
                transaction
                    .commit_or_rollback(write_result, |error| error)
                    .map(|_| ())
            }
            Err(error) => Err(error),
        };
        let mut delivery = DeliveryReport::default();
        delivery.record(DeliveryFailureKind::Storage, persist_result);
        if let Some(export_batch) = export_batch {
            delivery.extend(
                crate::RecordingWriter::new(self.storage).export_semantic_action_batches(
                    self.export_runtime,
                    traces,
                    export_batch,
                    emitted_at,
                    next_diagnostic_id,
                ),
            );
        }
        delivery
    }

    pub(crate) fn write_session_then_export(
        &mut self,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
        export_batch: SemanticActionBatch,
        observe_semantic_flush: impl FnOnce(Duration),
        write: impl FnOnce(&mut ObservedRecordWriteSession<'_>) -> Result<(), RecordingError>,
    ) -> DeliveryReport {
        let (persist_result, semantic_flush_elapsed) =
            match RecordingTransaction::begin(self.storage) {
                Ok(transaction) => {
                    let (write_result, semantic_flush_elapsed) = {
                        let mut session = ObservedRecordWriteSession::new(self.storage);
                        match write(&mut session) {
                            Ok(()) => {
                                let started = Instant::now();
                                let result = session.finish();
                                (result, Some(started.elapsed()))
                            }
                            Err(error) => (Err(error), None),
                        }
                    };
                    (
                        transaction
                            .commit_or_rollback(write_result, |error| error)
                            .map(|_| ()),
                        semantic_flush_elapsed,
                    )
                }
                Err(error) => (Err(error), None),
            };
        let mut delivery = DeliveryReport::default();
        delivery.record(DeliveryFailureKind::Storage, persist_result);
        if self.export_runtime.has_semantic_consumers() {
            delivery.extend(
                crate::RecordingWriter::new(self.storage).export_semantic_action_batches(
                    self.export_runtime,
                    traces,
                    export_batch,
                    emitted_at,
                    next_diagnostic_id,
                ),
            );
        }
        if let Some(elapsed) = semantic_flush_elapsed {
            observe_semantic_flush(elapsed);
        }
        delivery
    }

    fn export_without_persistence(
        &mut self,
        batch: SemanticActionBatch,
        traces: &dyn TraceRecordLookup,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> DeliveryReport {
        if !self.export_runtime.has_semantic_consumers() {
            return DeliveryReport::default();
        }
        crate::RecordingWriter::new(self.storage).export_semantic_action_batches(
            self.export_runtime,
            traces,
            batch,
            emitted_at,
            next_diagnostic_id,
        )
    }
}
