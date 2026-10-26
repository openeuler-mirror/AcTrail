use super::super::shared::FileSummaryPathAccumulator;
use crate::live::actions::event_action_id;
use config_core::daemon::{FileBulkReadMode, FileBulkReadObservationConfig};
use model_core::event::{DomainEvent, FileIoSummary};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    FilePathSetWrite, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionStatus, SemanticEvidence, SemanticEvidenceKind, attr_keys as attrs,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct BulkReadKey {
    pub(super) trace_id: TraceId,
    pub(super) process: ProcessIdentity,
}

/// Only the current delivery batch: no events, replay or cross-batch contribution cache.
pub(super) struct BulkReadState {
    action_id: String,
    trace_id: TraceId,
    process: ProcessIdentity,
    start_time: SystemTime,
    end_time: SystemTime,
    first_event_id: EventId,
    last_event_id: EventId,
    mode: FileBulkReadMode,
    paths: FileSummaryPathAccumulator,
    read_count: Option<u64>,
    bytes_read: Option<u64>,
    error_count: Option<u64>,
    has_error: bool,
    errnos: BTreeSet<u32>,
    complete: bool,
}
impl BulkReadState {
    pub(super) fn new(
        event: &DomainEvent,
        summary: &FileIoSummary,
        config: &FileBulkReadObservationConfig,
        counts_requested: bool,
        bytes_requested: bool,
    ) -> Self {
        Self {
            action_id: event_action_id(event, SemanticActionKind::FileBulkRead.as_str()),
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            start_time: summary.interval_start,
            end_time: summary.interval_end,
            first_event_id: event.envelope.event_id,
            last_event_id: event.envelope.event_id,
            mode: config.mode,
            paths: FileSummaryPathAccumulator::new(
                config.max_paths_per_set,
                config.path_set_chunk_max_paths,
            ),
            read_count: counts_requested.then_some(0),
            bytes_read: bytes_requested.then_some(0),
            error_count: counts_requested.then_some(0),
            has_error: false,
            errnos: BTreeSet::new(),
            complete: true,
        }
    }
    pub(super) fn observe(&mut self, event: &DomainEvent, summary: &FileIoSummary, path: &str) {
        self.start_time = self.start_time.min(summary.interval_start);
        self.end_time = self.end_time.max(summary.interval_end);
        self.last_event_id = event.envelope.event_id;
        self.complete &= summary.interval_complete;
        self.read_count = self
            .read_count
            .zip(summary.operations)
            .and_then(|(a, b)| a.checked_add(b));
        if summary.errno == 0 {
            self.bytes_read = self
                .bytes_read
                .zip(summary.bytes)
                .and_then(|(a, b)| a.checked_add(b));
            self.paths.record_path(path);
        } else {
            self.has_error = true;
            self.errnos.insert(summary.errno);
            self.error_count = self
                .error_count
                .zip(summary.operations)
                .and_then(|(a, b)| a.checked_add(b));
            self.paths.record_error_count(
                -(summary.errno as i32),
                path,
                summary.operations.unwrap_or(0),
            );
        }
    }
    pub(super) fn action(&self) -> SemanticAction {
        let mut attributes = BTreeMap::from([
            (
                attrs::file_bulk_read::MODE.to_string(),
                self.mode.as_str().to_string(),
            ),
            (
                attrs::file_bulk_read::UNIQUE_PATH_COUNT.to_string(),
                self.paths.stored_path_count().to_string(),
            ),
            (
                attrs::file_bulk_read::UNIQUE_PATH_COUNT_STATE.to_string(),
                self.paths.unique_path_count_state().to_string(),
            ),
            (
                attrs::file_bulk_read::STORED_PATH_COUNT.to_string(),
                self.paths.stored_path_count().to_string(),
            ),
            (
                attrs::file_bulk_read::PATH_OVERFLOW.to_string(),
                self.paths.path_overflow().to_string(),
            ),
            (
                attrs::file_bulk_read::FIRST_EVENT_ID.to_string(),
                self.first_event_id.get().to_string(),
            ),
            (
                attrs::file_bulk_read::LAST_EVENT_ID.to_string(),
                self.last_event_id.get().to_string(),
            ),
            (
                "file.interval_basis".to_string(),
                "collector_batch".to_string(),
            ),
            (
                "file.path_observation".to_string(),
                "first_observed".to_string(),
            ),
        ]);
        if let Some(count) = self.read_count {
            attributes.insert(
                attrs::file_bulk_read::READ_COUNT.to_string(),
                count.to_string(),
            );
        }
        if let Some(bytes) = self.bytes_read {
            attributes.insert(attrs::file::BYTES_READ.to_string(), bytes.to_string());
        }
        if self.has_error {
            attributes.insert(
                "file.errnos".to_string(),
                self.errnos
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
            if let Some(count) = self.error_count {
                attributes.insert(
                    attrs::file_bulk_read::ERROR_COUNT.to_string(),
                    count.to_string(),
                );
                if let Some(reasons) = self.paths.error_reason_counts_text() {
                    attributes.insert(
                        attrs::file_bulk_read::ERROR_REASON_COUNTS.to_string(),
                        reasons,
                    );
                }
            }
            attributes.insert(
                attrs::file_bulk_read::ERROR_STORED_PATH_COUNT.to_string(),
                self.paths.error_stored_path_count().to_string(),
            );
            attributes.insert(
                attrs::file_bulk_read::ERROR_PATH_OVERFLOW.to_string(),
                self.paths.error_path_overflow().to_string(),
            );
        }
        if self.mode == FileBulkReadMode::PathSet && self.paths.stored_path_count() > 0 {
            attributes.insert(
                attrs::file_bulk_read::PATH_SET_ID.to_string(),
                self.paths.path_set_id(None),
            );
            attributes.insert(
                attrs::file_bulk_read::PATH_SET_STATE.to_string(),
                self.paths.path_set_state().as_str().to_string(),
            );
            attributes.insert(
                attrs::file_bulk_read::CHUNKING_SCHEME.to_string(),
                self.paths.chunking_scheme(),
            );
        }
        let mut evidence = vec![self.evidence(self.first_event_id, "file.summary.first")];
        if self.last_event_id != self.first_event_id {
            evidence.push(self.evidence(self.last_event_id, "file.summary.last"));
        }
        SemanticAction {
            action_id: self.action_id.clone(),
            trace_id: self.trace_id,
            kind: SemanticActionKind::FileBulkRead,
            title: format!("bulk read {} paths", self.paths.stored_path_count()),
            start_time: self.start_time,
            end_time: Some(self.end_time),
            process: self.process.clone(),
            status: if self.has_error {
                SemanticActionStatus::Error
            } else {
                SemanticActionStatus::Success
            },
            completeness: if self.complete {
                SemanticActionCompleteness::Complete
            } else {
                SemanticActionCompleteness::Partial
            },
            attributes,
            evidence,
        }
    }
    fn evidence(&self, id: EventId, role: &str) -> SemanticEvidence {
        SemanticEvidence {
            kind: SemanticEvidenceKind::Event,
            id: id.get(),
            role: role.to_string(),
        }
    }
    pub(super) fn path_set_write(&self) -> Vec<FilePathSetWrite> {
        if self.mode != FileBulkReadMode::PathSet {
            return Vec::new();
        }
        self.paths
            .path_set_write(self.trace_id, &self.action_id, None)
    }
}
