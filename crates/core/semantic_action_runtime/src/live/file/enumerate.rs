use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{FileRawEventRetention, FsEnumerateObservationConfig};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    FilePathSetState, FilePathSetWrite, SemanticAction, SemanticActionCompleteness,
    SemanticActionKind, SemanticActionStatus, attr_keys as attrs,
};

use super::common::{event_fd, event_result};
use crate::live::actions::{event_action_id, status_from_result};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FsEnumerateOutput {
    pub(super) actions: Vec<SemanticAction>,
    pub(super) file_path_sets: Vec<FilePathSetWrite>,
    pub(super) handled_event: bool,
    pub(super) consumed_by_summary: bool,
    pub(super) retain_event: bool,
}

impl Default for FsEnumerateOutput {
    fn default() -> Self {
        Self {
            actions: Vec::new(),
            file_path_sets: Vec::new(),
            handled_event: false,
            consumed_by_summary: false,
            retain_event: true,
        }
    }
}

pub(super) struct FsEnumerateProjector {
    config: FsEnumerateObservationConfig,
    bursts: BTreeMap<FsEnumerateKey, FsEnumerateState>,
    open_dirs: BTreeMap<FsEnumerateFdKey, String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FsEnumerateKey {
    trace_id: TraceId,
    process: ProcessIdentity,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FsEnumerateFdKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    fd: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FsEnumerateOperation {
    Open,
    Close,
}

struct FsEnumerateEvent {
    operation: FsEnumerateOperation,
    path: String,
}

impl FsEnumerateProjector {
    pub(super) fn new(config: FsEnumerateObservationConfig) -> Self {
        Self {
            config,
            bursts: BTreeMap::new(),
            open_dirs: BTreeMap::new(),
        }
    }

    pub(super) fn observe(&mut self, event: &DomainEvent) -> FsEnumerateOutput {
        if !self.config.enabled {
            return FsEnumerateOutput::default();
        }
        let Some(enumerate_event) = self.enumerate_event(event) else {
            return FsEnumerateOutput::default();
        };
        let key = FsEnumerateKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
        };
        let state = self.bursts.entry(key).or_insert_with(|| {
            FsEnumerateState::new(
                event,
                &enumerate_event.path,
                self.config.max_paths_per_set,
                self.config.path_set_chunk_max_paths,
            )
        });
        let was_active = state.active;
        state.observe(event, enumerate_event);
        if !state.active && state.stored_path_count() >= u64::from(self.config.min_unique_paths) {
            state.active = true;
        }
        let actions = if !was_active && state.active {
            vec![state.action(
                event.envelope.observed_at,
                SemanticActionCompleteness::Partial,
            )]
        } else {
            Vec::new()
        };
        let consumed_by_summary = state.active;
        FsEnumerateOutput {
            actions,
            file_path_sets: Vec::new(),
            handled_event: true,
            consumed_by_summary,
            retain_event: if consumed_by_summary {
                should_retain_event(self.config.raw_event_retention, event)
            } else {
                true
            },
        }
    }

    pub(super) fn observe_boundary(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        observed_at: SystemTime,
    ) -> FsEnumerateOutput {
        if !self.config.enabled {
            return FsEnumerateOutput::default();
        }
        let key = FsEnumerateKey {
            trace_id,
            process: process.clone(),
        };
        let Some(state) = self.bursts.remove(&key) else {
            return FsEnumerateOutput::default();
        };
        self.open_dirs
            .retain(|fd_key, _| fd_key.trace_id != trace_id || fd_key.process != *process);
        if !state.active {
            return FsEnumerateOutput::default();
        }
        FsEnumerateOutput {
            actions: vec![state.action(observed_at, SemanticActionCompleteness::Complete)],
            file_path_sets: state.path_set_write(),
            handled_event: false,
            consumed_by_summary: false,
            retain_event: true,
        }
    }

    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> FsEnumerateOutput {
        let mut output = FsEnumerateOutput::default();
        self.open_dirs.retain(|key, _| key.trace_id != trace_id);
        self.bursts.retain(|key, state| {
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
        self.bursts.retain(|key, _| key.trace_id != trace_id);
        self.open_dirs.retain(|key, _| key.trace_id != trace_id);
    }

    fn enumerate_event(&mut self, event: &DomainEvent) -> Option<FsEnumerateEvent> {
        let EventPayload::File(payload) = &event.payload else {
            return None;
        };
        let fd_key = event_fd(event).map(|fd| FsEnumerateFdKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        });
        match payload.operation.as_str() {
            "open" if open_has_directory_flag(payload) => {
                let path = payload.path.clone()?;
                if event_result(event).is_some_and(|result| result >= 0) {
                    if let Some(fd_key) = fd_key {
                        self.open_dirs.insert(fd_key, path.clone());
                    }
                }
                Some(FsEnumerateEvent {
                    operation: FsEnumerateOperation::Open,
                    path,
                })
            }
            "close" => {
                let path = fd_key.and_then(|fd_key| self.open_dirs.remove(&fd_key))?;
                Some(FsEnumerateEvent {
                    operation: FsEnumerateOperation::Close,
                    path,
                })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FsEnumerateState {
    action_id: String,
    trace_id: TraceId,
    process: ProcessIdentity,
    start_time: SystemTime,
    first_event_id: EventId,
    last_event_id: EventId,
    active: bool,
    first_path: String,
    max_paths_per_set: u32,
    path_set_chunk_max_paths: u32,
    path_order_by_path: BTreeMap<String, u32>,
    path_overflow: bool,
    open_count: u64,
    close_count: u64,
    error_count: u64,
}

impl FsEnumerateState {
    fn new(
        event: &DomainEvent,
        first_path: &str,
        max_paths_per_set: u32,
        path_set_chunk_max_paths: u32,
    ) -> Self {
        Self {
            action_id: event_action_id(event, SemanticActionKind::FsEnumerate.as_str()),
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            start_time: event.envelope.observed_at,
            first_event_id: event.envelope.event_id,
            last_event_id: event.envelope.event_id,
            active: false,
            first_path: first_path.to_string(),
            max_paths_per_set,
            path_set_chunk_max_paths,
            path_order_by_path: BTreeMap::new(),
            path_overflow: false,
            open_count: 0,
            close_count: 0,
            error_count: 0,
        }
    }

    fn observe(&mut self, event: &DomainEvent, enumerate_event: FsEnumerateEvent) {
        self.last_event_id = event.envelope.event_id;
        if event_result(event).is_some_and(|result| result < 0) {
            self.error_count = self.error_count.saturating_add(1);
        }
        match enumerate_event.operation {
            FsEnumerateOperation::Open => {
                self.open_count = self.open_count.saturating_add(1);
                self.record_path(&enumerate_event.path);
            }
            FsEnumerateOperation::Close => {
                self.close_count = self.close_count.saturating_add(1);
            }
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
        let attributes = BTreeMap::from([
            (
                attrs::fs_enumerate::OPEN_COUNT.to_string(),
                self.open_count.to_string(),
            ),
            (
                attrs::fs_enumerate::CLOSE_COUNT.to_string(),
                self.close_count.to_string(),
            ),
            (
                attrs::fs_enumerate::ERROR_COUNT.to_string(),
                self.error_count.to_string(),
            ),
            (
                attrs::fs_enumerate::UNIQUE_PATH_COUNT.to_string(),
                self.stored_path_count().to_string(),
            ),
            (
                attrs::fs_enumerate::UNIQUE_PATH_COUNT_STATE.to_string(),
                unique_path_count_state.to_string(),
            ),
            (
                attrs::fs_enumerate::STORED_PATH_COUNT.to_string(),
                self.stored_path_count().to_string(),
            ),
            (
                attrs::fs_enumerate::PATH_OVERFLOW.to_string(),
                self.path_overflow.to_string(),
            ),
            (
                attrs::fs_enumerate::FIRST_EVENT_ID.to_string(),
                self.first_event_id.get().to_string(),
            ),
            (
                attrs::fs_enumerate::LAST_EVENT_ID.to_string(),
                self.last_event_id.get().to_string(),
            ),
            (
                attrs::fs_enumerate::PATH_SET_ID.to_string(),
                self.path_set_id(),
            ),
            (
                attrs::fs_enumerate::PATH_SET_STATE.to_string(),
                path_set_state.as_str().to_string(),
            ),
            (
                attrs::fs_enumerate::CHUNKING_SCHEME.to_string(),
                chunking_scheme_for(self.path_set_chunk_max_paths),
            ),
            (attrs::file::PATH.to_string(), self.first_path.clone()),
        ]);
        SemanticAction {
            action_id: self.action_id.clone(),
            trace_id: self.trace_id,
            kind: SemanticActionKind::FsEnumerate,
            title: self.first_path.clone(),
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

fn open_has_directory_flag(payload: &model_core::event::FilePayload) -> bool {
    let Some(flags) = payload
        .metadata
        .get("flags")
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return false;
    };
    flags & libc::O_DIRECTORY as u64 != 0
}

fn chunking_scheme_for(chunk_max_paths: u32) -> String {
    format!("path-id-v1:chunk-max={chunk_max_paths}")
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
