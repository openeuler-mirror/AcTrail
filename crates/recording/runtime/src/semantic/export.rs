use std::collections::BTreeSet;
use std::time::SystemTime;

use export_core::{ExportRuntime, SemanticActionExportBatch};
use model_core::ids::{DiagnosticId, TraceId};
use model_core::trace::TraceRecord;
use semantic_action::{SemanticAction, SemanticActionKind, SemanticActionLink};
use storage_core::StorageBackend;

use crate::diagnostics::export_drop_diagnostics;

use super::{RecordingError, SemanticActionBatch, SemanticActionRecordBatch};

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
        let report = if batch.actions().iter().all(action_exportable) {
            self.export_runtime
                .publish_semantic_actions(SemanticActionExportBatch {
                    trace,
                    actions: batch.actions(),
                    links: batch.links(),
                })?
        } else {
            let exportable_actions = exportable_actions(batch.actions());
            if exportable_actions.is_empty() {
                return Ok(());
            }
            let exportable_links = exportable_links(&exportable_actions, batch.links());
            self.export_runtime
                .publish_semantic_actions(SemanticActionExportBatch {
                    trace,
                    actions: &exportable_actions,
                    links: &exportable_links,
                })?
        };
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

    pub(crate) fn publish_batches_by_trace(
        &mut self,
        traces: &dyn TraceRecordLookup,
        semantic_actions: SemanticActionBatch,
        emitted_at: SystemTime,
        mut next_diagnostic_id: impl FnMut() -> Result<DiagnosticId, RecordingError>,
    ) -> Result<(), RecordingError> {
        for batch in semantic_actions.split_by_trace() {
            self.publish_batch_for_trace(
                traces,
                batch.as_record_batch(),
                emitted_at,
                &mut next_diagnostic_id,
            )?;
        }
        Ok(())
    }
}

fn action_exportable(action: &SemanticAction) -> bool {
    action.kind != SemanticActionKind::FileTtyIo
}

fn exportable_actions(actions: &[SemanticAction]) -> Vec<SemanticAction> {
    actions
        .iter()
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
