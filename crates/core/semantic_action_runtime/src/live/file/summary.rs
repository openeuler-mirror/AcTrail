use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{FileObservationConfig, FileRawEventRetention};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    FileObservationPath, FilePathSetWrite, SemanticAction, SemanticActionCompleteness,
    SemanticActionStatus,
};

use super::bulk_read::{BulkReadKey, BulkReadState, bulk_read_operation_candidate};
use super::common::event_result;
use super::tty::{TtyKey, TtyState};
use crate::live::actions::status_from_result;

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
        if !state.active() {
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
            if state.active() {
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
        let was_active = state.active();
        state.observe(event, operation, path, &self.config.bulk_read);
        let activates_now = !state.active()
            && state.stored_path_count() >= u64::from(self.config.bulk_read.min_unique_paths);
        if !was_active && !activates_now {
            state.record_pending_read_invalidation(event, operation, path);
        }
        if activates_now {
            state.activate();
        }
        if !state.active() {
            return FileSummaryOutput::default();
        }
        let actions = if was_active {
            Vec::new()
        } else {
            let mut actions = state.take_pending_read_invalidations();
            actions.push(state.action(
                event.envelope.observed_at,
                SemanticActionCompleteness::Partial,
            ));
            actions
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

fn should_retain_event(retention: FileRawEventRetention, event: &DomainEvent) -> bool {
    match status_from_result(event_result(event)) {
        SemanticActionStatus::Error => retention.retains_error(),
        _ => retention.retains_success(),
    }
}
