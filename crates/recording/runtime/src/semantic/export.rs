use std::collections::BTreeSet;
use std::time::SystemTime;

use crate::DeliveryFailureKind;
use export_core::{ExportRuntime, SemanticActionExportBatch};
use model_core::ids::{DiagnosticId, TraceId};
use model_core::trace::TraceRecord;
use semantic_action::{
    FileObservationPath, SemanticAction, SemanticActionKind, SemanticActionLink,
};

use super::{
    RecordingError, SemanticActionBatch, SemanticActionPublication, SemanticActionRecordBatch,
};

const LIVE_EXPORT_STAGE: &str = "live_export";

pub trait TraceRecordLookup {
    fn trace_record(&self, trace_id: TraceId) -> Option<&TraceRecord>;
}

pub(crate) struct SemanticActionExportRecorder<'a> {
    export_runtime: &'a ExportRuntime,
}

impl<'a> SemanticActionExportRecorder<'a> {
    pub(crate) fn new(export_runtime: &'a ExportRuntime) -> Self {
        Self { export_runtime }
    }

    pub(crate) fn publish_batch(
        &mut self,
        trace: &TraceRecord,
        batch: SemanticActionRecordBatch<'_>,
        trace_finalized: bool,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> SemanticActionPublication {
        let mut delivery = SemanticActionPublication::default();
        if !self.export_runtime.has_semantic_consumers() {
            return delivery;
        }
        if batch.action_views().next().is_none() && !trace_finalized {
            return delivery;
        }
        let exportable_actions = exportable_actions(batch.action_views());
        if exportable_actions.is_empty() && !trace_finalized {
            return delivery;
        }
        let exportable_links = exportable_links(&exportable_actions, batch.links());
        let exportable_paths =
            exportable_paths(&exportable_actions, batch.file_observation_paths());
        let payload_refs = SemanticActionPublication::payload_references(
            &exportable_actions,
            batch.payload_segments(),
        );
        let publish_result = self
            .export_runtime
            .publish_semantic_actions(SemanticActionExportBatch {
                trace,
                trace_finalized,
                actions: &exportable_actions,
                links: &exportable_links,
                file_observation_paths: &exportable_paths,
                payload_refs: &payload_refs,
            })
            .map_err(RecordingError::from);
        match publish_result {
            Ok(report) => {
                // Export backpressure is recorded after publish so collection can continue visibly.
                match crate::diagnostics::export_drop_diagnostics(
                    report,
                    emitted_at,
                    next_diagnostic_id,
                ) {
                    Ok(diagnostics) => delivery.diagnostics.extend(diagnostics),
                    Err(error) => delivery.record(DeliveryFailureKind::Runtime, Err(error)),
                }
            }
            Err(error) => delivery.record(DeliveryFailureKind::Consumer, Err(error)),
        }
        delivery
    }

    pub(crate) fn publish_batch_for_trace(
        &mut self,
        traces: &dyn TraceRecordLookup,
        batch: SemanticActionRecordBatch<'_>,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> SemanticActionPublication {
        let mut delivery = SemanticActionPublication::default();
        if !self.export_runtime.has_semantic_consumers() {
            return delivery;
        }
        if batch.action_views().next().is_none() {
            return delivery;
        }
        let trace_id = match batch.trace_id().and_then(|trace_id| {
            trace_id.ok_or_else(|| {
                RecordingError::new(LIVE_EXPORT_STAGE, "empty semantic action batch")
            })
        }) {
            Ok(trace_id) => trace_id,
            Err(error) => {
                delivery.record(DeliveryFailureKind::Runtime, Err(error));
                return delivery;
            }
        };
        let Some(trace) = traces.trace_record(trace_id) else {
            delivery.record(
                DeliveryFailureKind::Runtime,
                Err(RecordingError::new(LIVE_EXPORT_STAGE, "trace not found")),
            );
            return delivery;
        };
        self.publish_batch(trace, batch, false, emitted_at, next_diagnostic_id)
    }

    pub(crate) fn publish_final_batch_for_trace(
        &mut self,
        traces: &dyn TraceRecordLookup,
        trace_id: TraceId,
        batch: SemanticActionRecordBatch<'_>,
        emitted_at: SystemTime,
        next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> SemanticActionPublication {
        let mut delivery = SemanticActionPublication::default();
        if !self.export_runtime.has_semantic_consumers() {
            return delivery;
        }
        let batch_trace_id = match batch.trace_id() {
            Ok(trace_id) => trace_id,
            Err(error) => {
                delivery.record(DeliveryFailureKind::Runtime, Err(error));
                return delivery;
            }
        };
        if let Some(batch_trace_id) = batch_trace_id
            && batch_trace_id != trace_id
        {
            delivery.record(
                DeliveryFailureKind::Runtime,
                Err(RecordingError::new(
                    LIVE_EXPORT_STAGE,
                    "final semantic action batch trace_id does not match finalized trace",
                )),
            );
            return delivery;
        }
        let Some(trace) = traces.trace_record(trace_id) else {
            delivery.record(
                DeliveryFailureKind::Runtime,
                Err(RecordingError::new(LIVE_EXPORT_STAGE, "trace not found")),
            );
            return delivery;
        };
        self.publish_batch(trace, batch, true, emitted_at, next_diagnostic_id)
    }

    pub(crate) fn publish_batches_by_trace(
        &mut self,
        traces: &dyn TraceRecordLookup,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        mut next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> SemanticActionPublication {
        let mut delivery = SemanticActionPublication::default();
        if !self.export_runtime.has_semantic_consumers() {
            return delivery;
        }
        for batch in semantic_actions.split_by_trace() {
            delivery.extend(self.publish_batch_for_trace(
                traces,
                batch.as_record_batch(),
                emitted_at,
                &mut next_diagnostic_id,
            ));
        }
        delivery
    }
}

pub(super) fn action_exportable(action: &SemanticAction) -> bool {
    action.kind != SemanticActionKind::FileTtyIo
}

fn exportable_actions<'a>(
    actions: impl Iterator<Item = &'a SemanticAction>,
) -> Vec<SemanticAction> {
    actions
        .filter(|action| action_exportable(action))
        .cloned()
        .collect()
}

fn exportable_links(
    exportable_actions: &[SemanticAction],
    links: &[SemanticActionLink],
) -> Vec<SemanticActionLink> {
    let exportable_child_ids = exportable_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect::<BTreeSet<_>>();
    links
        .iter()
        .filter(|link| exportable_child_ids.contains(link.child_action_id.as_str()))
        .cloned()
        .collect()
}

fn exportable_paths(
    exportable_actions: &[SemanticAction],
    paths: &[FileObservationPath],
) -> Vec<FileObservationPath> {
    let exportable_action_ids = exportable_actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect::<BTreeSet<_>>();
    paths
        .iter()
        .filter(|path| exportable_action_ids.contains(path.action_id.as_str()))
        .cloned()
        .collect()
}
