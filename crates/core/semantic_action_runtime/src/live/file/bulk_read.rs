use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{FileBulkReadMode, FileBulkReadObservationConfig};
use model_core::event::DomainEvent;
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    FilePathSetState, FilePathSetWrite, SemanticAction, SemanticActionCompleteness,
    SemanticActionKind, SemanticActionStatus, attr_keys as attrs,
};

use super::common::{FileSummaryPathAccumulator, event_fd, event_result, event_size};
use crate::live::actions::event_action_id;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct BulkReadKey {
    pub(super) trace_id: TraceId,
    pub(super) process: ProcessIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BulkReadState {
    action_id: String,
    trace_id: TraceId,
    process: ProcessIdentity,
    start_time: SystemTime,
    first_event_id: EventId,
    last_event_id: EventId,
    active: bool,
    mode: FileBulkReadMode,
    paths: FileSummaryPathAccumulator,
    open_event_by_fd: BTreeMap<u32, EventId>,
    pending_events: Vec<DomainEvent>,
    open_count: u64,
    close_count: u64,
    read_count: u64,
    bytes_read: u64,
}

impl BulkReadState {
    pub(super) fn new(
        event: &DomainEvent,
        mode: FileBulkReadMode,
        max_paths_per_set: u32,
        path_set_chunk_max_paths: u32,
    ) -> Self {
        Self {
            action_id: event_action_id(event, SemanticActionKind::FileBulkRead.as_str()),
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            start_time: event.envelope.observed_at,
            first_event_id: event.envelope.event_id,
            last_event_id: event.envelope.event_id,
            active: false,
            mode,
            paths: FileSummaryPathAccumulator::new(max_paths_per_set, path_set_chunk_max_paths),
            open_event_by_fd: BTreeMap::new(),
            pending_events: Vec::new(),
            open_count: 0,
            close_count: 0,
            read_count: 0,
            bytes_read: 0,
        }
    }

    pub(super) fn active(&self) -> bool {
        self.active
    }

    pub(super) fn activate(&mut self) {
        self.active = true;
    }

    pub(super) fn observe(
        &mut self,
        event: &DomainEvent,
        operation: &str,
        path: &str,
        config: &FileBulkReadObservationConfig,
    ) {
        self.last_event_id = event.envelope.event_id;
        if let Some(result) = event_result(event).filter(|result| *result < 0) {
            self.paths.record_error(result, path);
        }
        match operation {
            "open" => {
                self.open_count = self.open_count.saturating_add(1);
                if event_result(event).is_none_or(|result| result >= 0)
                    && let Some(fd) = event_fd(event)
                {
                    self.open_event_by_fd.insert(fd, event.envelope.event_id);
                }
            }
            "close" => {
                self.close_count = self.close_count.saturating_add(1);
                if let Some(fd) = event_fd(event) {
                    self.open_event_by_fd.remove(&fd);
                }
            }
            "read" | "readv" => {
                self.read_count = self.read_count.saturating_add(1);
                self.bytes_read = self
                    .bytes_read
                    .saturating_add(event_size(event).unwrap_or(0));
                if event_result(event).is_some_and(|result| result < 0) {
                    return;
                }
                if config.mode == FileBulkReadMode::PathSet {
                    self.paths.record_path(path);
                    return;
                }
                self.paths.record_path(path);
            }
            _ => {}
        }
    }

    pub(super) fn record_pending_event(&mut self, event: &DomainEvent) {
        self.pending_events.push(event.clone());
    }

    pub(super) fn pending_event_count(&self) -> usize {
        self.pending_events.len()
    }

    pub(super) fn take_pending_events(&mut self) -> Vec<DomainEvent> {
        std::mem::take(&mut self.pending_events)
    }

    pub(super) fn action(
        &self,
        end_time: SystemTime,
        completeness: SemanticActionCompleteness,
    ) -> SemanticAction {
        let path_set_state = if completeness == SemanticActionCompleteness::Partial {
            FilePathSetState::Pending
        } else {
            self.paths.path_set_state()
        };
        let mut attributes = BTreeMap::from([
            (
                attrs::file_bulk_read::MODE.to_string(),
                self.mode.as_str().to_string(),
            ),
            (
                attrs::file_bulk_read::OPEN_COUNT.to_string(),
                self.open_count.to_string(),
            ),
            (
                attrs::file_bulk_read::CLOSE_COUNT.to_string(),
                self.close_count.to_string(),
            ),
            (
                attrs::file_bulk_read::READ_COUNT.to_string(),
                self.read_count.to_string(),
            ),
            (
                attrs::file::BYTES_READ.to_string(),
                self.bytes_read.to_string(),
            ),
            (
                attrs::file_bulk_read::ERROR_COUNT.to_string(),
                self.paths.error_count().to_string(),
            ),
            (
                attrs::file_bulk_read::UNIQUE_PATH_COUNT.to_string(),
                self.stored_path_count().to_string(),
            ),
            (
                attrs::file_bulk_read::UNIQUE_PATH_COUNT_STATE.to_string(),
                self.paths.unique_path_count_state().to_string(),
            ),
            (
                attrs::file_bulk_read::STORED_PATH_COUNT.to_string(),
                self.stored_path_count().to_string(),
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
        ]);
        if self.mode == FileBulkReadMode::PathSet {
            attributes.insert(
                attrs::file_bulk_read::PATH_SET_ID.to_string(),
                self.paths.path_set_id(None),
            );
            attributes.insert(
                attrs::file_bulk_read::PATH_SET_STATE.to_string(),
                path_set_state.as_str().to_string(),
            );
            attributes.insert(
                attrs::file_bulk_read::CHUNKING_SCHEME.to_string(),
                self.paths.chunking_scheme(),
            );
        }
        if let Some(error_reason_counts) = self.paths.error_reason_counts_text() {
            attributes.insert(
                attrs::file_bulk_read::ERROR_REASON_COUNTS.to_string(),
                error_reason_counts,
            );
            attributes.insert(
                attrs::file_bulk_read::ERROR_UNIQUE_PATH_COUNT.to_string(),
                self.error_stored_path_count().to_string(),
            );
            attributes.insert(
                attrs::file_bulk_read::ERROR_UNIQUE_PATH_COUNT_STATE.to_string(),
                self.paths.error_unique_path_count_state().to_string(),
            );
            attributes.insert(
                attrs::file_bulk_read::ERROR_STORED_PATH_COUNT.to_string(),
                self.error_stored_path_count().to_string(),
            );
            attributes.insert(
                attrs::file_bulk_read::ERROR_PATH_OVERFLOW.to_string(),
                self.paths.error_path_overflow().to_string(),
            );
        }
        SemanticAction {
            action_id: self.action_id.clone(),
            trace_id: self.trace_id,
            kind: SemanticActionKind::FileBulkRead,
            title: format!("bulk read {} paths", self.stored_path_count()),
            start_time: self.start_time,
            end_time: Some(end_time),
            process: self.process.clone(),
            status: SemanticActionStatus::Success,
            completeness,
            confidence_millis: None,
            attributes,
            evidence: Vec::new(),
        }
    }

    pub(super) fn stored_path_count(&self) -> u64 {
        self.paths.stored_path_count()
    }

    fn error_stored_path_count(&self) -> u64 {
        self.paths.error_stored_path_count()
    }

    pub(super) fn should_activate(&self, min_unique_paths: u32) -> bool {
        self.stored_path_count() >= u64::from(min_unique_paths)
            || (self.paths.path_overflow()
                && self.stored_path_count() > 0
                && self.read_count >= u64::from(min_unique_paths))
    }

    pub(super) fn path_set_write(&self) -> Vec<FilePathSetWrite> {
        if self.mode != FileBulkReadMode::PathSet {
            return Vec::new();
        }
        if self.stored_path_count() == 0 {
            return Vec::new();
        }
        self.paths
            .path_set_write(self.trace_id, &self.action_id, None)
    }
}

pub(super) fn bulk_read_operation_candidate(operation: &str) -> bool {
    matches!(operation, "open" | "close" | "read" | "readv")
}
