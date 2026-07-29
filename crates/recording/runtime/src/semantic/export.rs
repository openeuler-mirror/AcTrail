use std::collections::BTreeSet;
use std::time::SystemTime;

use export_core::{ExportRuntime, SemanticActionExportBatch};
use model_core::ids::{DiagnosticId, TraceId};
use model_core::payload::PayloadSegment;
use model_core::trace::TraceRecord;
use semantic_action::{
    FileObservationPath, SemanticAction, SemanticActionKind, SemanticActionLink,
};
use storage_core::StorageBackend;
use storage_core::{PayloadRowLimit, PayloadSegmentQuery};

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
        let exportable_actions = exportable_actions(batch.actions());
        if exportable_actions.is_empty() {
            return Ok(());
        }
        let exportable_links = exportable_links(&exportable_actions, batch.links());
        let exportable_paths =
            exportable_paths(&exportable_actions, batch.file_observation_paths());
        let payload_snapshot = self.payload_segments_for_export(trace.trace_id);
        let payload_segments = payload_snapshot.as_deref().unwrap_or_default();
        let publish_result = self
            .export_runtime
            .publish_semantic_actions(SemanticActionExportBatch {
                trace,
                actions: &exportable_actions,
                links: &exportable_links,
                file_observation_paths: &exportable_paths,
                payload_segments,
            })
            .map_err(RecordingError::from)
            .and_then(|report| {
                // Export backpressure is recorded after publish so collection can continue visibly.
                crate::writer::RecordingWriter::new(self.storage).persist_export_drop_report(
                    report,
                    emitted_at,
                    next_diagnostic_id,
                )
            });
        combine_snapshot_and_publish(payload_snapshot.map(|_| ()), publish_result)
    }

    fn payload_segments_for_export(
        &self,
        trace_id: TraceId,
    ) -> Result<Vec<PayloadSegment>, RecordingError> {
        let Some(limit) = self.export_runtime.payload_snapshot_limit() else {
            return Ok(Vec::new());
        };
        self.storage
            .list_payload_segments(
                trace_id,
                PayloadSegmentQuery {
                    segment_id: None,
                    direction: None,
                    limit: Some(PayloadRowLimit::Head(limit)),
                    include_bytes: true,
                },
            )
            .map_err(|error| RecordingError::new(error.stage, error.message))
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
        let mut errors = Vec::new();
        for batch in semantic_actions.split_by_trace() {
            if let Err(error) = self.publish_batch_for_trace(
                traces,
                batch.as_record_batch(),
                emitted_at,
                &mut next_diagnostic_id,
            ) {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            return Ok(());
        }
        Err(RecordingError::new(
            LIVE_EXPORT_STAGE,
            errors
                .into_iter()
                .map(|error| format!("{}: {}", error.stage, error.message))
                .collect::<Vec<_>>()
                .join("; "),
        ))
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

fn combine_snapshot_and_publish(
    snapshot_result: Result<(), RecordingError>,
    publish_result: Result<(), RecordingError>,
) -> Result<(), RecordingError> {
    match (snapshot_result, publish_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(snapshot), Err(publish)) => Err(RecordingError::new(
            LIVE_EXPORT_STAGE,
            format!(
                "payload snapshot failed at {}: {}; publish failed at {}: {}",
                snapshot.stage, snapshot.message, publish.stage, publish.message
            ),
        )),
    }
}
