use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{FileRawEventRetention, FsEnumerateObservationConfig};
use model_core::event::DomainEvent;
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    FilePathSetState, FilePathSetWrite, SemanticAction, SemanticActionCompleteness,
    SemanticActionKind, SemanticActionStatus, attr_keys as attrs,
};

use super::super::shared::{FileSummaryPathAccumulator, event_result};
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
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FsEnumerateKey {
    trace_id: TraceId,
    process: ProcessIdentity,
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
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.config.enabled
    }

    pub(super) fn observe_open(&mut self, event: &DomainEvent, path: String) -> FsEnumerateOutput {
        self.observe_owned_event(
            event,
            FsEnumerateEvent {
                operation: FsEnumerateOperation::Open,
                path,
            },
        )
    }

    pub(super) fn observe_close(&mut self, event: &DomainEvent, path: String) -> FsEnumerateOutput {
        self.observe_owned_event(
            event,
            FsEnumerateEvent {
                operation: FsEnumerateOperation::Close,
                path,
            },
        )
    }

    fn observe_owned_event(
        &mut self,
        event: &DomainEvent,
        enumerate_event: FsEnumerateEvent,
    ) -> FsEnumerateOutput {
        if !self.config.enabled {
            return FsEnumerateOutput::default();
        }
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
        if !state.active && state.should_activate(self.config.min_unique_paths) {
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
    first_path: Option<String>,
    paths: FileSummaryPathAccumulator,
    open_count: u64,
    close_count: u64,
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
            first_path: if event_result(event).is_none_or(|result| result >= 0) {
                Some(first_path.to_string())
            } else {
                None
            },
            paths: FileSummaryPathAccumulator::new(max_paths_per_set, path_set_chunk_max_paths),
            open_count: 0,
            close_count: 0,
        }
    }

    fn observe(&mut self, event: &DomainEvent, enumerate_event: FsEnumerateEvent) {
        self.last_event_id = event.envelope.event_id;
        let result = event_result(event);
        if let Some(error) = result.filter(|result| *result < 0) {
            self.paths.record_error(error, &enumerate_event.path);
        }
        match enumerate_event.operation {
            FsEnumerateOperation::Open => {
                self.open_count = self.open_count.saturating_add(1);
                if result.is_none_or(|result| result >= 0) {
                    if self.first_path.is_none() {
                        self.first_path = Some(enumerate_event.path.clone());
                    }
                    self.paths.record_path(&enumerate_event.path);
                }
            }
            FsEnumerateOperation::Close => {
                self.close_count = self.close_count.saturating_add(1);
            }
        }
    }

    fn action(
        &self,
        end_time: SystemTime,
        completeness: SemanticActionCompleteness,
    ) -> SemanticAction {
        let path_set_state = if completeness == SemanticActionCompleteness::Partial {
            FilePathSetState::Pending
        } else {
            self.paths.path_set_state()
        };
        let title = self
            .first_path
            .clone()
            .unwrap_or_else(|| "fs.enumerate".to_string());
        let mut attributes = BTreeMap::from([
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
                self.paths.error_count().to_string(),
            ),
            (
                attrs::fs_enumerate::UNIQUE_PATH_COUNT.to_string(),
                self.stored_path_count().to_string(),
            ),
            (
                attrs::fs_enumerate::UNIQUE_PATH_COUNT_STATE.to_string(),
                self.paths.unique_path_count_state().to_string(),
            ),
            (
                attrs::fs_enumerate::STORED_PATH_COUNT.to_string(),
                self.stored_path_count().to_string(),
            ),
            (
                attrs::fs_enumerate::PATH_OVERFLOW.to_string(),
                self.paths.path_overflow().to_string(),
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
                self.paths.path_set_id(self.first_path.as_deref()),
            ),
            (
                attrs::fs_enumerate::PATH_SET_STATE.to_string(),
                path_set_state.as_str().to_string(),
            ),
            (
                attrs::fs_enumerate::CHUNKING_SCHEME.to_string(),
                self.paths.chunking_scheme(),
            ),
            (attrs::file::PATH.to_string(), title.clone()),
        ]);
        if let Some(error_reason_counts) = self.paths.error_reason_counts_text() {
            attributes.insert(
                attrs::fs_enumerate::ERROR_REASON_COUNTS.to_string(),
                error_reason_counts,
            );
            attributes.insert(
                attrs::fs_enumerate::ERROR_UNIQUE_PATH_COUNT.to_string(),
                self.paths.error_stored_path_count().to_string(),
            );
            attributes.insert(
                attrs::fs_enumerate::ERROR_UNIQUE_PATH_COUNT_STATE.to_string(),
                self.paths.error_unique_path_count_state().to_string(),
            );
            attributes.insert(
                attrs::fs_enumerate::ERROR_STORED_PATH_COUNT.to_string(),
                self.paths.error_stored_path_count().to_string(),
            );
            attributes.insert(
                attrs::fs_enumerate::ERROR_PATH_OVERFLOW.to_string(),
                self.paths.error_path_overflow().to_string(),
            );
        }
        SemanticAction {
            action_id: self.action_id.clone(),
            trace_id: self.trace_id,
            kind: SemanticActionKind::FsEnumerate,
            title,
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

    fn stored_path_count(&self) -> u64 {
        self.paths.stored_path_count()
    }

    fn should_activate(&self, min_unique_paths: u32) -> bool {
        self.stored_path_count() >= u64::from(min_unique_paths)
            || (self.paths.path_overflow()
                && self.stored_path_count() > 0
                && self.open_count >= u64::from(min_unique_paths))
    }

    fn path_set_write(&self) -> Vec<FilePathSetWrite> {
        self.paths
            .path_set_write(self.trace_id, &self.action_id, self.first_path.as_deref())
    }
}

fn should_retain_event(retention: FileRawEventRetention, event: &DomainEvent) -> bool {
    match status_from_result(event_result(event)) {
        SemanticActionStatus::Error => retention.retains_error(),
        _ => retention.retains_success(),
    }
}
