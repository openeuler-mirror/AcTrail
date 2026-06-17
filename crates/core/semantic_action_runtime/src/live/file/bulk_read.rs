use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{FileBulkReadMode, FileBulkReadObservationConfig};
use model_core::event::DomainEvent;
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    FilePathSetState, FilePathSetWrite, SemanticAction, SemanticActionCompleteness,
    SemanticActionKind, SemanticActionStatus, attr_keys as attrs, evidence_roles,
};

use super::common::{event_fd, event_result, event_size};
use crate::live::actions::{
    event_action_id, event_action_id_for_event_id, event_evidence, status_from_result,
};

const VALID_FALSE: &str = "false";

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
    max_paths_per_set: u32,
    path_set_chunk_max_paths: u32,
    path_order_by_path: BTreeMap<String, u32>,
    path_overflow: bool,
    open_event_by_fd: BTreeMap<u32, EventId>,
    pending_read_invalidations: BTreeMap<String, SemanticAction>,
    open_count: u64,
    close_count: u64,
    read_count: u64,
    bytes_read: u64,
    error_count: u64,
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
            max_paths_per_set,
            path_set_chunk_max_paths,
            path_order_by_path: BTreeMap::new(),
            path_overflow: false,
            open_event_by_fd: BTreeMap::new(),
            pending_read_invalidations: BTreeMap::new(),
            open_count: 0,
            close_count: 0,
            read_count: 0,
            bytes_read: 0,
            error_count: 0,
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
        if event_result(event).is_some_and(|result| result < 0) {
            self.error_count = self.error_count.saturating_add(1);
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
                if config.mode == FileBulkReadMode::PathSet {
                    self.record_path(path);
                    return;
                }
                self.record_path(path);
            }
            _ => {}
        }
    }

    pub(super) fn record_pending_read_invalidation(
        &mut self,
        event: &DomainEvent,
        operation: &str,
        path: &str,
    ) {
        if !matches!(operation, "read" | "readv") {
            return;
        }
        let action_id = self.detailed_read_action_id(event);
        let incoming = invalidated_read_action(event, &action_id, path);
        self.pending_read_invalidations
            .entry(action_id)
            .and_modify(|existing| {
                for evidence in &incoming.evidence {
                    if !existing.evidence.contains(evidence) {
                        existing.evidence.push(evidence.clone());
                    }
                }
                existing.end_time = existing.end_time.max(incoming.end_time);
            })
            .or_insert(incoming);
    }

    pub(super) fn take_pending_read_invalidations(&mut self) -> Vec<SemanticAction> {
        std::mem::take(&mut self.pending_read_invalidations)
            .into_values()
            .collect()
    }

    pub(super) fn action(
        &self,
        end_time: SystemTime,
        completeness: SemanticActionCompleteness,
    ) -> SemanticAction {
        let unique_path_count_state = if self.path_overflow {
            "lower_bound"
        } else {
            "exact"
        };
        let path_set_state = if completeness == SemanticActionCompleteness::Partial {
            FilePathSetState::Pending
        } else {
            self.path_set_state()
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
                self.error_count.to_string(),
            ),
            (
                attrs::file_bulk_read::UNIQUE_PATH_COUNT.to_string(),
                self.stored_path_count().to_string(),
            ),
            (
                attrs::file_bulk_read::UNIQUE_PATH_COUNT_STATE.to_string(),
                unique_path_count_state.to_string(),
            ),
            (
                attrs::file_bulk_read::STORED_PATH_COUNT.to_string(),
                self.stored_path_count().to_string(),
            ),
            (
                attrs::file_bulk_read::PATH_OVERFLOW.to_string(),
                self.path_overflow.to_string(),
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
                self.path_set_id(),
            );
            attributes.insert(
                attrs::file_bulk_read::PATH_SET_STATE.to_string(),
                path_set_state.as_str().to_string(),
            );
            attributes.insert(
                attrs::file_bulk_read::CHUNKING_SCHEME.to_string(),
                chunking_scheme_for(self.path_set_chunk_max_paths),
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
            status: aggregate_status(self.error_count),
            completeness,
            confidence_millis: None,
            attributes,
            evidence: Vec::new(),
        }
    }

    pub(super) fn stored_path_count(&self) -> u64 {
        self.path_order_by_path.len() as u64
    }

    pub(super) fn path_set_write(&self) -> Vec<FilePathSetWrite> {
        if self.mode != FileBulkReadMode::PathSet {
            return Vec::new();
        }
        vec![FilePathSetWrite {
            trace_id: self.trace_id,
            action_id: self.action_id.clone(),
            path_set_id: self.path_set_id(),
            state: self.path_set_state(),
            unique_path_count: self.stored_path_count(),
            stored_path_count: self.stored_path_count(),
            chunking_scheme: chunking_scheme_for(self.path_set_chunk_max_paths),
            chunk_max_paths: self.path_set_chunk_max_paths,
            paths: self.path_order_by_path.keys().cloned().collect(),
        }]
    }

    fn record_path(&mut self, path: &str) {
        if self.path_order_by_path.contains_key(path) {
            return;
        }
        if self.path_order_by_path.len() >= self.max_paths_per_set as usize {
            self.path_overflow = true;
            return;
        }
        let path_order = self.path_order_by_path.len() as u32;
        self.path_order_by_path.insert(path.to_string(), path_order);
    }

    fn detailed_read_action_id(&self, event: &DomainEvent) -> String {
        event_fd(event)
            .and_then(|fd| self.open_event_by_fd.get(&fd).copied())
            .map(|open_event_id| {
                event_action_id_for_event_id(
                    event.envelope.trace_id,
                    open_event_id,
                    SemanticActionKind::FileRead.as_str(),
                )
            })
            .unwrap_or_else(|| event_action_id(event, SemanticActionKind::FileRead.as_str()))
    }

    fn path_set_id(&self) -> String {
        format!("{}:path_set", self.action_id)
    }

    fn path_set_state(&self) -> FilePathSetState {
        if self.path_overflow {
            FilePathSetState::Overflow
        } else {
            FilePathSetState::Complete
        }
    }
}

pub(super) fn bulk_read_operation_candidate(operation: &str) -> bool {
    matches!(operation, "open" | "close" | "read" | "readv")
}

fn invalidated_read_action(event: &DomainEvent, action_id: &str, path: &str) -> SemanticAction {
    let mut attributes = BTreeMap::from([
        (attrs::file::PATH.to_string(), path.to_string()),
        (
            attrs::actrail::ACTION_VALID.to_string(),
            VALID_FALSE.to_string(),
        ),
    ]);
    if let Some(fd) = event_fd(event) {
        attributes.insert(attrs::file::FD.to_string(), fd.to_string());
    }
    SemanticAction {
        action_id: action_id.to_string(),
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::FileRead,
        title: path.to_string(),
        start_time: event.envelope.observed_at,
        end_time: Some(event.envelope.observed_at),
        process: event.envelope.process.clone(),
        status: status_from_result(event_result(event)),
        completeness: SemanticActionCompleteness::Partial,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(event, evidence_roles::file::READ)],
    }
}

fn chunking_scheme_for(chunk_max_paths: u32) -> String {
    format!("path-id-v1:chunk-max={chunk_max_paths}")
}

fn aggregate_status(error_count: u64) -> SemanticActionStatus {
    if error_count == 0 {
        SemanticActionStatus::Success
    } else {
        SemanticActionStatus::Error
    }
}
