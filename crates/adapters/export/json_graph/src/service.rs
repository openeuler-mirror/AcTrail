//! Export orchestration from snapshots to graph documents.

use graph_contract::document::GraphDocument;
use model_core::ids::TraceId;
use storage_core::{SnapshotView, StorageBackend};

use crate::document::build_graph_document;
use crate::serialize::to_json;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportError {
    pub stage: String,
    pub message: String,
}

impl ExportError {
    pub fn new(stage: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            stage: stage.into(),
            message: message.into(),
        }
    }
}

pub struct JsonGraphExportService<'a> {
    storage: &'a mut dyn StorageBackend,
    schema_version: String,
    include_payload_bytes: bool,
    include_payload_text: bool,
}

impl<'a> JsonGraphExportService<'a> {
    pub fn new(
        storage: &'a mut dyn StorageBackend,
        schema_version: impl Into<String>,
        include_payload_bytes: bool,
        include_payload_text: bool,
    ) -> Self {
        Self {
            storage,
            schema_version: schema_version.into(),
            include_payload_bytes,
            include_payload_text,
        }
    }

    pub fn export_json(&mut self, trace_id: TraceId) -> Result<String, ExportError> {
        let snapshot = self.read_snapshot_with_lease(trace_id)?;
        let document = build_graph_document(
            self.schema_version.clone(),
            snapshot,
            self.include_payload_bytes,
            self.include_payload_text,
        );
        Ok(to_json(&document))
    }

    pub fn export_document(&mut self, trace_id: TraceId) -> Result<GraphDocument, ExportError> {
        let snapshot = self.read_snapshot_with_lease(trace_id)?;
        Ok(build_graph_document(
            self.schema_version.clone(),
            snapshot,
            self.include_payload_bytes,
            self.include_payload_text,
        ))
    }

    fn read_snapshot_with_lease(&mut self, trace_id: TraceId) -> Result<SnapshotView, ExportError> {
        let lease = self
            .storage
            .acquire_export_lease(trace_id)
            .map_err(|error| ExportError::new(error.stage, error.message))?;
        let snapshot_result = self
            .storage
            .read_snapshot(&lease)
            .map_err(|error| ExportError::new(error.stage, error.message));
        let release_result = self
            .storage
            .release_export_lease(lease)
            .map_err(|error| ExportError::new(error.stage, error.message));

        match (snapshot_result, release_result) {
            (Ok(snapshot), Ok(())) => Ok(snapshot),
            (_, Err(error)) => Err(error),
            (Err(error), Ok(())) => Err(error),
        }
    }
}
