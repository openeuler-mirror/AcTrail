//! File access projection from file syscall events.

use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::FileObservationConfig;
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{SemanticAction, SemanticActionCompleteness};

use super::super::shared::{
    FileFdOwner, FileFdRegistry, event_error_count, event_fd, event_file_path,
    event_read_summary_count, event_result, event_size, event_source_fd, event_target_fd,
    file_open_has_directory_flag,
};
use super::bulk_read::bulk_read_operation_candidate;
use super::enumerate::{FsEnumerateOutput, FsEnumerateProjector};
use super::io_action::{
    FileAccessKind, FileIoState, OpenFileActionContext, single_io_action, terminal_io_action,
};
use super::summary::{FileSummaryOutput, FileSummaryProjector};
use crate::live::actions::{is_file_modify_event, is_file_modify_operation, status_from_result};
use crate::live::runtime::LiveSemanticActionOutput;

pub(in crate::live) struct FileAccessProjector {
    enumerate: FsEnumerateProjector,
    summary: FileSummaryProjector,
    fd_registry: FileFdRegistry,
    open_files: BTreeMap<FileHandleKey, FileHandleState>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileHandleKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    fd: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileHandleState {
    trace_id: TraceId,
    process: ProcessIdentity,
    action_seed_event_id: EventId,
    open_event_id: EventId,
    open_evidence_role: Option<&'static str>,
    open_time: SystemTime,
    path: String,
    read: FileIoState,
    write: FileIoState,
}

impl FileAccessProjector {
    pub(in crate::live) fn new(config: FileObservationConfig) -> Self {
        Self {
            enumerate: FsEnumerateProjector::new(config.enumerate.clone()),
            summary: FileSummaryProjector::new(config),
            fd_registry: FileFdRegistry::default(),
            open_files: BTreeMap::new(),
        }
    }

    pub(in crate::live) fn observe_file_event(
        &mut self,
        event: &DomainEvent,
    ) -> LiveSemanticActionOutput {
        let EventPayload::File(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        if fd_duplicate_lifecycle_operation(&payload.operation) {
            let pending = self.summary.release_pending_before_fd_lifecycle(event);
            let mut output = self.live_output_from_summary(pending, false);
            append_output(&mut output, self.observe_file_duplicate(event));
            self.fd_registry.duplicate(event);
            if !event_result(event).is_some_and(|result| result < 0) {
                output.retain_event = false;
                output.raw_event_consumed = true;
            }
            return output;
        }
        if payload.operation == "open" && file_open_has_directory_flag(payload) {
            if let Some(output) = self.observe_directory_open(event) {
                return output;
            }
        }
        if payload.operation == "close" {
            if let Some(output) = self.observe_owned_close(event) {
                return output;
            }
        }
        let mut output = LiveSemanticActionOutput::default();
        if completes_enumerate_boundary(&payload.operation) || is_file_modify_event(event) {
            append_output(
                &mut output,
                live_output_from_enumerate(
                    self.enumerate.observe_boundary(
                        event.envelope.trace_id,
                        &event.envelope.process,
                        event.envelope.observed_at,
                    ),
                    false,
                ),
            );
        }
        let summary = self.summary.observe(event);
        let summary_consumed = summary.consumed_by_summary;
        append_output(
            &mut output,
            self.live_output_from_summary(summary, summary_consumed),
        );
        if summary_consumed {
            return output;
        }
        let lifecycle = match payload.operation.as_str() {
            "open" => self.observe_open(event),
            "close" => self.observe_close(event),
            _ => LiveSemanticActionOutput::default(),
        };
        append_output(&mut output, lifecycle);
        let current = match payload.operation.as_str() {
            "open" | "close" => LiveSemanticActionOutput::default(),
            "read" | "readv" | "read_summary" => self.observe_io(event, FileAccessKind::Read),
            "write" | "writev" => self.observe_io(event, FileAccessKind::Write),
            _ => LiveSemanticActionOutput::default(),
        };
        append_output(&mut output, current);
        consume_successful_close(&mut output, event);
        consume_successful_unprojectable_file_event(&mut output, event);
        output
    }

    pub(in crate::live) fn observe_boundary(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        observed_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let summary = self
            .summary
            .observe_boundary(trace_id, process, observed_at);
        let mut output = self.live_output_from_summary(summary, false);
        append_output(
            &mut output,
            live_output_from_enumerate(
                self.enumerate
                    .observe_boundary(trace_id, process, observed_at),
                false,
            ),
        );
        output
    }

    pub(in crate::live) fn observe_boundary_for_event(
        &mut self,
        event: &DomainEvent,
    ) -> LiveSemanticActionOutput {
        let mut output = self.observe_boundary(
            event.envelope.trace_id,
            &event.envelope.process,
            event.envelope.observed_at,
        );
        if matches!(
            &event.payload,
            EventPayload::Process(payload) if payload.operation == "exit"
        ) {
            append_output(
                &mut output,
                self.finalize_process_handles(
                    event.envelope.trace_id,
                    &event.envelope.process,
                    event.envelope.observed_at,
                ),
            );
            self.fd_registry
                .forget_process(event.envelope.trace_id, &event.envelope.process);
        }
        if !matches!(event.payload, EventPayload::File(_)) {
            output.retain_event = true;
            output.raw_event_consumed = false;
        }
        output
    }

    pub(in crate::live) fn forget_trace(&mut self, trace_id: TraceId) {
        self.enumerate.forget_trace(trace_id);
        self.summary.forget_trace(trace_id);
        self.fd_registry.forget_trace(trace_id);
        self.open_files.retain(|key, _| key.trace_id != trace_id);
    }

    pub(in crate::live) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let summary = self.summary.finalize_trace(trace_id, finished_at);
        let mut output = self.live_output_from_summary(summary, false);
        append_output(
            &mut output,
            live_output_from_enumerate(self.enumerate.finalize_trace(trace_id, finished_at), false),
        );
        append_output(
            &mut output,
            self.finalize_trace_handles(trace_id, finished_at),
        );
        output
    }

    fn observe_directory_open(&mut self, event: &DomainEvent) -> Option<LiveSemanticActionOutput> {
        if !self.enumerate.enabled() {
            return None;
        }
        let path = event_file_path(event)?;
        if !event_result(event).is_some_and(|result| result < 0) {
            if let Some(fd) = event_fd(event) {
                self.fd_registry
                    .insert(event, fd, FileFdOwner::FsEnumerate, path.clone());
            }
        }
        let enumerate = self.enumerate.observe_open(event, path);
        Some(live_output_from_enumerate(
            enumerate.clone(),
            enumerate.consumed_by_summary,
        ))
    }

    fn observe_owned_close(&mut self, event: &DomainEvent) -> Option<LiveSemanticActionOutput> {
        let fd = event_fd(event)?;
        let state = self.fd_registry.close_state(event, fd)?;
        match state.owner {
            FileFdOwner::FsEnumerate => {
                let enumerate = self.enumerate.observe_close(event, state.path);
                Some(live_output_from_enumerate(
                    enumerate.clone(),
                    enumerate.consumed_by_summary,
                ))
            }
        }
    }

    fn live_output_from_summary(
        &mut self,
        summary: FileSummaryOutput,
        raw_event_consumed: bool,
    ) -> LiveSemanticActionOutput {
        let mut output = LiveSemanticActionOutput {
            actions: summary.actions,
            ..LiveSemanticActionOutput::default()
        };
        output.file_observation_paths = summary.file_observation_paths;
        output.file_path_sets = summary.file_path_sets;
        output.deferred_events = summary.deferred_events;
        output.retain_event = summary.retain_event;
        output.raw_event_consumed = raw_event_consumed;
        for event in summary.released_detailed_events {
            append_replayed_output(&mut output, self.observe_released_detailed_event(&event));
        }
        output
    }

    fn observe_released_detailed_event(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let EventPayload::File(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        match payload.operation.as_str() {
            "open" => self.observe_open(event),
            "read" | "readv" | "read_summary" => self.observe_io(event, FileAccessKind::Read),
            "write" | "writev" => self.observe_io(event, FileAccessKind::Write),
            "close" => self.observe_close(event),
            _ => LiveSemanticActionOutput::default(),
        }
    }

    fn observe_open(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let Some(fd) = event_fd(event) else {
            return LiveSemanticActionOutput::default();
        };
        if event_result(event).is_some_and(|result| result < 0) {
            return LiveSemanticActionOutput::default();
        }
        let Some(path) = event_file_path(event) else {
            return LiveSemanticActionOutput::default();
        };
        let key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        };
        let replaced = self.open_files.remove(&key);
        self.open_files.insert(
            key,
            FileHandleState {
                trace_id: event.envelope.trace_id,
                process: event.envelope.process.clone(),
                action_seed_event_id: event.envelope.event_id,
                open_event_id: event.envelope.event_id,
                open_evidence_role: Some(semantic_action::evidence_roles::file::OPEN),
                open_time: event.envelope.observed_at,
                path,
                read: FileIoState::default(),
                write: FileIoState::default(),
            },
        );
        let actions = replaced
            .map(|state| {
                state.terminal_actions(
                    fd,
                    event.envelope.observed_at,
                    SemanticActionCompleteness::Partial,
                    None,
                )
            })
            .unwrap_or_default();
        LiveSemanticActionOutput {
            actions,
            ..LiveSemanticActionOutput::default()
        }
    }

    fn observe_io(
        &mut self,
        event: &DomainEvent,
        kind: FileAccessKind,
    ) -> LiveSemanticActionOutput {
        let bytes = event_size(event).unwrap_or_default();
        let count = event_read_summary_count(event).unwrap_or(1);
        let status = status_from_result(event_result(event));
        let error_count = event_error_count(event).unwrap_or_else(|| {
            if status == semantic_action::SemanticActionStatus::Error {
                count.max(1)
            } else {
                0
            }
        });
        let Some(fd) = event_fd(event) else {
            let Some(path) = event_file_path(event) else {
                return LiveSemanticActionOutput::default();
            };
            let action = single_io_action(event, kind, &path, bytes, count, error_count, status);
            let mut output = LiveSemanticActionOutput {
                actions: vec![action],
                ..LiveSemanticActionOutput::default()
            };
            output.retain_event = true;
            output.raw_event_consumed = false;
            return output;
        };
        let key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        };
        if let Some(state) = self.open_files.get_mut(&key) {
            state
                .io_mut(kind)
                .observe(event.envelope.event_id, bytes, count, error_count);
            return LiveSemanticActionOutput {
                actions: Vec::new(),
                retain_event: true,
                raw_event_consumed: false,
                ..LiveSemanticActionOutput::default()
            };
        }
        let Some(path) = event_file_path(event) else {
            return LiveSemanticActionOutput::default();
        };
        let action = single_io_action(event, kind, &path, bytes, count, error_count, status);
        let mut output = LiveSemanticActionOutput {
            actions: vec![action],
            ..LiveSemanticActionOutput::default()
        };
        output.retain_event = true;
        output.raw_event_consumed = false;
        output
    }

    fn observe_file_duplicate(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        if event_result(event).is_some_and(|result| result < 0) {
            return LiveSemanticActionOutput::default();
        }
        let Some(source_fd) = event_source_fd(event) else {
            return LiveSemanticActionOutput::default();
        };
        let Some(target_fd) = event_target_fd(event) else {
            return LiveSemanticActionOutput::default();
        };
        if source_fd == target_fd {
            return LiveSemanticActionOutput::default();
        }
        let source_key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd: source_fd,
        };
        let target_key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd: target_fd,
        };
        let source_state = self.open_files.get(&source_key).cloned();
        let mut actions = self
            .open_files
            .remove(&target_key)
            .map(|state| {
                state.terminal_actions(
                    target_fd,
                    event.envelope.observed_at,
                    SemanticActionCompleteness::Partial,
                    None,
                )
            })
            .unwrap_or_default();
        if let Some(source_state) = source_state {
            self.open_files.insert(
                target_key,
                FileHandleState {
                    trace_id: event.envelope.trace_id,
                    process: event.envelope.process.clone(),
                    action_seed_event_id: event.envelope.event_id,
                    open_event_id: event.envelope.event_id,
                    open_evidence_role: None,
                    open_time: event.envelope.observed_at,
                    path: source_state.path,
                    read: FileIoState::default(),
                    write: FileIoState::default(),
                },
            );
        }
        LiveSemanticActionOutput {
            actions: std::mem::take(&mut actions),
            ..LiveSemanticActionOutput::default()
        }
    }

    fn observe_close(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let Some(fd) = event_fd(event) else {
            return LiveSemanticActionOutput::default();
        };
        if event_result(event).is_some_and(|result| result < 0) {
            return LiveSemanticActionOutput::default();
        }
        let key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        };
        let Some(state) = self.open_files.remove(&key) else {
            return LiveSemanticActionOutput::default();
        };
        let actions = state.terminal_actions(
            fd,
            event.envelope.observed_at,
            SemanticActionCompleteness::Complete,
            Some(event.envelope.event_id),
        );
        let mut output = LiveSemanticActionOutput {
            actions,
            ..LiveSemanticActionOutput::default()
        };
        output.retain_event = false;
        output.raw_event_consumed = true;
        output
    }

    fn finalize_process_handles(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let keys = self
            .open_files
            .keys()
            .filter(|key| key.trace_id == trace_id && key.process == *process)
            .cloned()
            .collect::<Vec<_>>();
        self.finalize_handle_keys(keys, finished_at)
    }

    fn finalize_trace_handles(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let keys = self
            .open_files
            .keys()
            .filter(|key| key.trace_id == trace_id)
            .cloned()
            .collect::<Vec<_>>();
        self.finalize_handle_keys(keys, finished_at)
    }

    fn finalize_handle_keys(
        &mut self,
        keys: Vec<FileHandleKey>,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let mut actions = Vec::new();
        for key in keys {
            let Some(state) = self.open_files.remove(&key) else {
                continue;
            };
            actions.extend(state.terminal_actions(
                key.fd,
                finished_at,
                SemanticActionCompleteness::Partial,
                None,
            ));
        }
        LiveSemanticActionOutput {
            actions,
            ..LiveSemanticActionOutput::default()
        }
    }
}

fn completes_enumerate_boundary(operation: &str) -> bool {
    matches!(
        operation,
        "write" | "writev" | "truncate" | "unlink" | "rename" | "mkdir" | "rmdir" | "mmap_shared"
    ) && !bulk_read_operation_candidate(operation)
}

fn fd_duplicate_lifecycle_operation(operation: &str) -> bool {
    matches!(operation, "dup" | "dup2" | "dup3" | "fcntl_dup")
}

fn consume_successful_close(output: &mut LiveSemanticActionOutput, event: &DomainEvent) {
    let EventPayload::File(payload) = &event.payload else {
        return;
    };
    if payload.operation == "close" && !event_result(event).is_some_and(|result| result < 0) {
        output.retain_event = false;
        output.raw_event_consumed = true;
    }
}

fn consume_successful_unprojectable_file_event(
    output: &mut LiveSemanticActionOutput,
    event: &DomainEvent,
) {
    let EventPayload::File(payload) = &event.payload else {
        return;
    };
    if is_file_modify_operation(&payload.operation) {
        return;
    }
    if event_result(event).is_some_and(|result| result < 0) {
        return;
    }
    if event_file_path(event).is_some() {
        return;
    }
    output.retain_event = false;
    output.raw_event_consumed = true;
}

fn live_output_from_enumerate(
    enumerate: FsEnumerateOutput,
    raw_event_consumed: bool,
) -> LiveSemanticActionOutput {
    let mut output = LiveSemanticActionOutput {
        actions: enumerate.actions,
        ..LiveSemanticActionOutput::default()
    };
    output.file_path_sets = enumerate.file_path_sets;
    output.retain_event = enumerate.retain_event;
    output.raw_event_consumed = raw_event_consumed;
    output
}

fn append_output(output: &mut LiveSemanticActionOutput, other: LiveSemanticActionOutput) {
    output.actions.extend(other.actions);
    output.links.extend(other.links);
    output
        .file_observation_paths
        .extend(other.file_observation_paths);
    output.file_path_sets.extend(other.file_path_sets);
    output
        .llm_request_contents
        .extend(other.llm_request_contents);
    output
        .llm_pipeline_diagnostics
        .extend(other.llm_pipeline_diagnostics);
    output.deferred_events.extend(other.deferred_events);
    output.retain_event = output.retain_event && other.retain_event;
    output.raw_event_consumed = output.raw_event_consumed || other.raw_event_consumed;
}

fn append_replayed_output(output: &mut LiveSemanticActionOutput, other: LiveSemanticActionOutput) {
    output.actions.extend(other.actions);
    output.links.extend(other.links);
    output
        .file_observation_paths
        .extend(other.file_observation_paths);
    output.file_path_sets.extend(other.file_path_sets);
    output
        .llm_request_contents
        .extend(other.llm_request_contents);
    output
        .llm_pipeline_diagnostics
        .extend(other.llm_pipeline_diagnostics);
    output.deferred_events.extend(other.deferred_events);
}

impl FileHandleState {
    fn io_mut(&mut self, kind: FileAccessKind) -> &mut FileIoState {
        match kind {
            FileAccessKind::Read => &mut self.read,
            FileAccessKind::Write => &mut self.write,
        }
    }

    fn io(&self, kind: FileAccessKind) -> &FileIoState {
        match kind {
            FileAccessKind::Read => &self.read,
            FileAccessKind::Write => &self.write,
        }
    }

    fn terminal_actions(
        &self,
        fd: u32,
        end_time: SystemTime,
        completeness: SemanticActionCompleteness,
        close_event_id: Option<EventId>,
    ) -> Vec<SemanticAction> {
        [FileAccessKind::Read, FileAccessKind::Write]
            .into_iter()
            .filter_map(|kind| {
                terminal_io_action(
                    OpenFileActionContext {
                        trace_id: self.trace_id,
                        process: &self.process,
                        fd,
                        action_seed_event_id: self.action_seed_event_id,
                        open_event_id: self.open_event_id,
                        open_evidence_role: self.open_evidence_role,
                        open_time: self.open_time,
                        path: &self.path,
                    },
                    kind,
                    self.io(kind),
                    end_time,
                    completeness,
                    close_event_id,
                )
            })
            .collect()
    }
}
