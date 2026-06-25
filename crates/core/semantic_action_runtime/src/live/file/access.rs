//! File access projection from file syscall events.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use config_core::daemon::FileObservationConfig;
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionLink, SemanticActionLinkConfidence,
    SemanticActionLinkRole, evidence_roles,
};

use super::bulk_read::bulk_read_operation_candidate;
use super::common::{
    event_fd, event_file_path, event_result, event_size, file_open_has_directory_flag,
};
use super::enumerate::{FsEnumerateOutput, FsEnumerateProjector};
use super::fd::{FileFdOwner, FileFdRegistry};
use super::io::{
    FileAccessKind, FileIoState, complete_close_action, open_backed_io_action, single_io_action,
};
use super::summary::{FileSummaryOutput, FileSummaryProjector};
use crate::live::actions::{event_evidence, is_file_modify_operation, status_from_result};
use crate::live::runtime::LiveSemanticActionOutput;

pub(in crate::live) struct FileAccessProjector {
    enumerate: FsEnumerateProjector,
    summary: FileSummaryProjector,
    fd_registry: FileFdRegistry,
    open_files: BTreeMap<FileHandleKey, FileHandleState>,
    linked_file_events: BTreeSet<FileEventLinkKey>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileHandleKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    fd: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileHandleState {
    open_event_id: EventId,
    open_time: SystemTime,
    path: String,
    read: FileIoState,
    write: FileIoState,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileEventLinkKey {
    trace_id: TraceId,
    parent_action_id: String,
    child_action_id: String,
}

impl FileAccessProjector {
    pub(in crate::live) fn new(config: FileObservationConfig) -> Self {
        Self {
            enumerate: FsEnumerateProjector::new(config.enumerate.clone()),
            summary: FileSummaryProjector::new(config),
            fd_registry: FileFdRegistry::default(),
            open_files: BTreeMap::new(),
            linked_file_events: BTreeSet::new(),
        }
    }

    pub(in crate::live) fn observe_file_event(
        &mut self,
        event: &DomainEvent,
        file_modify_action: Option<&SemanticAction>,
    ) -> LiveSemanticActionOutput {
        let EventPayload::File(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        if fd_duplicate_lifecycle_operation(&payload.operation) {
            self.fd_registry.duplicate(event);
            return consumed_lifecycle_output();
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
        if completes_enumerate_boundary(&payload.operation) {
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
            consume_successful_close(&mut output, event);
            return output;
        }
        let current = match payload.operation.as_str() {
            "open" => {
                self.observe_open(event);
                LiveSemanticActionOutput::default()
            }
            "read" | "readv" => self.observe_io(event, FileAccessKind::Read, None),
            "write" | "writev" => self.observe_io(event, FileAccessKind::Write, file_modify_action),
            "close" => self.observe_close(event),
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
        self.linked_file_events
            .retain(|key| key.trace_id != trace_id);
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
            links: Vec::new(),
            file_observation_paths: summary.file_observation_paths,
            file_path_sets: summary.file_path_sets,
            llm_request_contents: Vec::new(),
            deferred_events: summary.deferred_events,
            retain_event: summary.retain_event,
            raw_event_consumed,
        };
        for event in summary.released_detailed_events {
            append_output(&mut output, self.observe_released_detailed_event(&event));
        }
        output
    }

    fn observe_released_detailed_event(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let EventPayload::File(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        match payload.operation.as_str() {
            "open" => {
                self.observe_open(event);
                LiveSemanticActionOutput::default()
            }
            "read" | "readv" => self.observe_io(event, FileAccessKind::Read, None),
            "write" | "writev" => self.observe_io(event, FileAccessKind::Write, None),
            "close" => self.observe_close(event),
            _ => LiveSemanticActionOutput::default(),
        }
    }

    fn observe_open(&mut self, event: &DomainEvent) {
        let Some(fd) = event_fd(event) else {
            return;
        };
        if event_result(event).is_some_and(|result| result < 0) {
            return;
        }
        let Some(path) = event_file_path(event) else {
            return;
        };
        let key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        };
        self.open_files.insert(
            key,
            FileHandleState {
                open_event_id: event.envelope.event_id,
                open_time: event.envelope.observed_at,
                path,
                read: FileIoState::default(),
                write: FileIoState::default(),
            },
        );
    }

    fn observe_io(
        &mut self,
        event: &DomainEvent,
        kind: FileAccessKind,
        file_modify_action: Option<&SemanticAction>,
    ) -> LiveSemanticActionOutput {
        let Some(path) = event_file_path(event) else {
            return LiveSemanticActionOutput::default();
        };
        let bytes = event_size(event).unwrap_or_default();
        let status = status_from_result(event_result(event));
        let Some(fd) = event_fd(event) else {
            let action = single_io_action(event, kind, &path, bytes, status);
            return LiveSemanticActionOutput {
                actions: vec![action.clone()],
                links: self.file_event_link(&action, kind, file_modify_action, event),
                file_observation_paths: Vec::new(),
                file_path_sets: Vec::new(),
                llm_request_contents: Vec::new(),
                deferred_events: Vec::new(),
                retain_event: true,
                raw_event_consumed: false,
            };
        };
        let key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        };
        let Some(mut state) = self.open_files.remove(&key) else {
            let action = single_io_action(event, kind, &path, bytes, status);
            return LiveSemanticActionOutput {
                actions: vec![action.clone()],
                links: self.file_event_link(&action, kind, file_modify_action, event),
                file_observation_paths: Vec::new(),
                file_path_sets: Vec::new(),
                llm_request_contents: Vec::new(),
                deferred_events: Vec::new(),
                retain_event: true,
                raw_event_consumed: false,
            };
        };
        let open_event_id = state.open_event_id;
        let open_time = state.open_time;
        let open_path = state.path.clone();
        let io = state.io_mut(kind);
        io.bytes = io.bytes.saturating_add(bytes);
        io.count = io.count.saturating_add(1);
        let mut action = io.action.clone().unwrap_or_else(|| {
            open_backed_io_action(event, kind, open_event_id, open_time, &open_path)
        });
        action.end_time = Some(event.envelope.observed_at);
        action.status = status;
        action.completeness = SemanticActionCompleteness::Partial;
        action
            .attributes
            .insert(kind.bytes_attr().to_string(), io.bytes.to_string());
        action
            .attributes
            .insert(kind.count_attr().to_string(), io.count.to_string());
        action
            .evidence
            .push(event_evidence(event, kind.event_role()));
        io.action = Some(action.clone());
        self.open_files.insert(key, state);
        LiveSemanticActionOutput {
            actions: vec![action.clone()],
            links: self.file_event_link(&action, kind, file_modify_action, event),
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            llm_request_contents: Vec::new(),
            deferred_events: Vec::new(),
            retain_event: true,
            raw_event_consumed: false,
        }
    }

    fn observe_close(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let Some(fd) = event_fd(event) else {
            return LiveSemanticActionOutput::default();
        };
        let key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        };
        let Some(mut state) = self.open_files.remove(&key) else {
            return LiveSemanticActionOutput::default();
        };
        let actions = [FileAccessKind::Read, FileAccessKind::Write]
            .into_iter()
            .filter_map(|kind| complete_close_action(state.io_mut(kind), event))
            .collect::<Vec<_>>();
        let retain_event = event_result(event).is_some_and(|result| result < 0);
        LiveSemanticActionOutput {
            actions,
            links: Vec::new(),
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            llm_request_contents: Vec::new(),
            deferred_events: Vec::new(),
            retain_event,
            raw_event_consumed: !retain_event,
        }
    }

    fn file_event_link(
        &mut self,
        action: &SemanticAction,
        kind: FileAccessKind,
        file_modify_action: Option<&SemanticAction>,
        event: &DomainEvent,
    ) -> Vec<SemanticActionLink> {
        if !matches!(kind, FileAccessKind::Write) {
            return Vec::new();
        }
        let Some(file_modify_action) = file_modify_action else {
            return Vec::new();
        };
        let key = FileEventLinkKey {
            trace_id: action.trace_id,
            parent_action_id: action.action_id.clone(),
            child_action_id: file_modify_action.action_id.clone(),
        };
        if !self.linked_file_events.insert(key) {
            return Vec::new();
        }
        vec![SemanticActionLink {
            trace_id: action.trace_id,
            parent_action_id: action.action_id.clone(),
            child_action_id: file_modify_action.action_id.clone(),
            role: SemanticActionLinkRole::FileWriteContainsFileEvent,
            confidence: SemanticActionLinkConfidence::Observed,
            valid: true,
            evidence: vec![event_evidence(event, evidence_roles::file::WRITE)],
            attributes: BTreeMap::new(),
        }]
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

fn consumed_lifecycle_output() -> LiveSemanticActionOutput {
    LiveSemanticActionOutput {
        actions: Vec::new(),
        links: Vec::new(),
        file_observation_paths: Vec::new(),
        file_path_sets: Vec::new(),
        llm_request_contents: Vec::new(),
        deferred_events: Vec::new(),
        retain_event: false,
        raw_event_consumed: true,
    }
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
    LiveSemanticActionOutput {
        actions: enumerate.actions,
        links: Vec::new(),
        file_observation_paths: Vec::new(),
        file_path_sets: enumerate.file_path_sets,
        llm_request_contents: Vec::new(),
        deferred_events: Vec::new(),
        retain_event: enumerate.retain_event,
        raw_event_consumed,
    }
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
    output.deferred_events.extend(other.deferred_events);
    output.retain_event = output.retain_event && other.retain_event;
    output.raw_event_consumed = output.raw_event_consumed || other.raw_event_consumed;
}

impl FileHandleState {
    fn io_mut(&mut self, kind: FileAccessKind) -> &mut FileIoState {
        match kind {
            FileAccessKind::Read => &mut self.read,
            FileAccessKind::Write => &mut self.write,
        }
    }
}
