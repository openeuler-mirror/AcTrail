use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{
    FileBulkReadMode, FileBulkReadObservationConfig, FileObservationConfig, FileRawEventRetention,
};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    FileObservationPath, FilePathSetState, FilePathSetWrite, SemanticAction,
    SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus, attr_keys as attrs,
};

use super::common::{event_result, event_size};
use super::tty::{TtyKey, TtyState};
use crate::live::actions::{event_action_id, status_from_result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FileSummaryOutput {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) file_observation_paths: Vec<FileObservationPath>,
    pub(super) file_path_sets: Vec<FilePathSetWrite>,
    pub(super) consumed_by_summary: bool,
    pub(super) retain_event: bool,
}

impl Default for FileSummaryOutput {
    fn default() -> Self {
        Self {
            actions: Vec::new(),
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            consumed_by_summary: false,
            retain_event: true,
        }
    }
}

impl FileSummaryOutput {
    fn extend(&mut self, other: Self) {
        self.actions.extend(other.actions);
        self.file_observation_paths
            .extend(other.file_observation_paths);
        self.file_path_sets.extend(other.file_path_sets);
        self.consumed_by_summary = self.consumed_by_summary || other.consumed_by_summary;
        self.retain_event = self.retain_event && other.retain_event;
    }
}

pub(super) struct FileSummaryProjector {
    config: FileObservationConfig,
    tty: BTreeMap<TtyKey, TtyState>,
    bulk_read: BTreeMap<BulkReadKey, BulkReadState>,
}

impl FileSummaryProjector {
    pub(super) fn new(config: FileObservationConfig) -> Self {
        Self {
            config,
            tty: BTreeMap::new(),
            bulk_read: BTreeMap::new(),
        }
    }

    pub(super) fn observe(&mut self, event: &DomainEvent) -> FileSummaryOutput {
        if !self.config.enabled {
            return FileSummaryOutput::default();
        }
        let EventPayload::File(payload) = &event.payload else {
            return FileSummaryOutput::default();
        };
        let Some(path) = payload.path.as_ref() else {
            return FileSummaryOutput::default();
        };
        if self.config.tty.matches(path, &payload.operation) {
            return self.observe_tty(event, &payload.operation, path);
        }
        let is_tty_path = self.config.tty.matches_path(path);
        let is_bulk_read_candidate = bulk_read_operation_candidate(&payload.operation);
        let mut output = if is_bulk_read_candidate || is_tty_path {
            FileSummaryOutput::default()
        } else {
            self.observe_boundary(
                event.envelope.trace_id,
                &event.envelope.process,
                event.envelope.observed_at,
            )
        };
        if is_bulk_read_candidate && !is_tty_path {
            output.extend(self.observe_bulk_read(event, &payload.operation, path));
        }
        output
    }

    pub(super) fn observe_boundary(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        observed_at: SystemTime,
    ) -> FileSummaryOutput {
        if !self.config.enabled || !self.config.bulk_read.enabled {
            return FileSummaryOutput::default();
        }
        let key = BulkReadKey {
            trace_id,
            process: process.clone(),
        };
        let Some(state) = self.bulk_read.remove(&key) else {
            return FileSummaryOutput::default();
        };
        if !state.active {
            return FileSummaryOutput::default();
        }
        FileSummaryOutput {
            actions: vec![state.action(observed_at, SemanticActionCompleteness::Complete)],
            file_observation_paths: Vec::new(),
            file_path_sets: state.path_set_write(),
            consumed_by_summary: false,
            retain_event: true,
        }
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> FileSummaryOutput {
        let mut output = FileSummaryOutput::default();
        self.tty.retain(|key, state| {
            if key.trace_id != trace_id {
                return true;
            }
            output
                .actions
                .push(state.action(finished_at, SemanticActionCompleteness::Complete));
            false
        });
        self.bulk_read.retain(|key, state| {
            if key.trace_id != trace_id {
                return true;
            }
            if state.active {
                output
                    .actions
                    .push(state.action(finished_at, SemanticActionCompleteness::Complete));
                output.file_path_sets.extend(state.path_set_write());
            }
            false
        });
        output
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.tty.retain(|key, _| key.trace_id != trace_id);
        self.bulk_read.retain(|key, _| key.trace_id != trace_id);
    }

    fn observe_tty(
        &mut self,
        event: &DomainEvent,
        operation: &str,
        path: &str,
    ) -> FileSummaryOutput {
        let key = TtyKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            path: path.to_string(),
        };
        let state = self
            .tty
            .entry(key)
            .or_insert_with(|| TtyState::new(event, path));
        state.observe(event, operation);
        FileSummaryOutput {
            actions: vec![state.action(
                event.envelope.observed_at,
                SemanticActionCompleteness::Partial,
            )],
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            consumed_by_summary: true,
            retain_event: should_retain_event(self.config.tty.raw_event_retention, event),
        }
    }

    fn observe_bulk_read(
        &mut self,
        event: &DomainEvent,
        operation: &str,
        path: &str,
    ) -> FileSummaryOutput {
        if !self.config.bulk_read.enabled || !bulk_read_operation_candidate(operation) {
            return FileSummaryOutput::default();
        }
        let key = BulkReadKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
        };
        let state = self.bulk_read.entry(key).or_insert_with(|| {
            BulkReadState::new(
                event,
                self.config.bulk_read.mode,
                self.config.bulk_read.max_paths_per_set,
                self.config.bulk_read.path_set_chunk_max_paths,
            )
        });
        let was_active = state.active;
        state.observe(event, operation, path, &self.config.bulk_read);
        if !state.active
            && state.stored_path_count() >= u64::from(self.config.bulk_read.min_unique_paths)
        {
            state.active = true;
        }
        if !state.active {
            return FileSummaryOutput::default();
        }
        let actions = if was_active {
            Vec::new()
        } else {
            vec![state.action(
                event.envelope.observed_at,
                SemanticActionCompleteness::Partial,
            )]
        };
        FileSummaryOutput {
            actions,
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            consumed_by_summary: true,
            retain_event: should_retain_event(self.config.bulk_read.raw_event_retention, event),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct BulkReadKey {
    trace_id: TraceId,
    process: ProcessIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BulkReadState {
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
    open_count: u64,
    close_count: u64,
    read_count: u64,
    bytes_read: u64,
    error_count: u64,
}

impl BulkReadState {
    fn new(
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
            open_count: 0,
            close_count: 0,
            read_count: 0,
            bytes_read: 0,
            error_count: 0,
        }
    }

    fn observe(
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
            "open" => self.open_count = self.open_count.saturating_add(1),
            "close" => self.close_count = self.close_count.saturating_add(1),
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

    fn action(
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

    fn stored_path_count(&self) -> u64 {
        self.path_order_by_path.len() as u64
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

    fn path_set_write(&self) -> Vec<FilePathSetWrite> {
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
}

fn chunking_scheme_for(chunk_max_paths: u32) -> String {
    format!("path-id-v1:chunk-max={chunk_max_paths}")
}

fn bulk_read_operation_candidate(operation: &str) -> bool {
    matches!(operation, "open" | "close" | "read" | "readv")
}

fn should_retain_event(retention: FileRawEventRetention, event: &DomainEvent) -> bool {
    match status_from_result(event_result(event)) {
        SemanticActionStatus::Error => retention.retains_error(),
        _ => retention.retains_success(),
    }
}

fn aggregate_status(error_count: u64) -> SemanticActionStatus {
    if error_count == 0 {
        SemanticActionStatus::Success
    } else {
        SemanticActionStatus::Error
    }
}
