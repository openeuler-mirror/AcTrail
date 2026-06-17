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

use super::common::{event_fd, event_result, event_size};
use super::enumerate::{FsEnumerateOutput, FsEnumerateProjector};
use super::io::{
    FileAccessKind, FileIoState, complete_close_action, open_backed_io_action, single_io_action,
};
use super::summary::{FileSummaryOutput, FileSummaryProjector};
use crate::live::actions::{event_evidence, status_from_result};
use crate::live::runtime::LiveSemanticActionOutput;

pub(in crate::live) struct FileAccessProjector {
    enumerate: FsEnumerateProjector,
    summary: FileSummaryProjector,
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
        let enumerate = self.enumerate.observe(event);
        if enumerate.handled_event {
            let mut output = live_output_from_summary(
                self.summary.observe_boundary(
                    event.envelope.trace_id,
                    &event.envelope.process,
                    event.envelope.observed_at,
                ),
                false,
            );
            let raw_event_consumed = enumerate.consumed_by_summary;
            append_output(
                &mut output,
                live_output_from_enumerate(enumerate, raw_event_consumed),
            );
            if output.raw_event_consumed {
                self.discard_open_state_for_event_fd(event);
            }
            return output;
        }
        let mut output = live_output_from_enumerate(
            self.enumerate.observe_boundary(
                event.envelope.trace_id,
                &event.envelope.process,
                event.envelope.observed_at,
            ),
            false,
        );
        let summary = self.summary.observe(event);
        if summary.consumed_by_summary {
            let raw_event_consumed = summary.consumed_by_summary;
            self.discard_open_state_for_event_fd(event);
            append_output(
                &mut output,
                live_output_from_summary(summary, raw_event_consumed),
            );
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
        output.actions.splice(0..0, summary.actions);
        output
            .file_observation_paths
            .extend(summary.file_observation_paths);
        output.file_path_sets.extend(summary.file_path_sets);
        output.retain_event = output.retain_event && summary.retain_event;
        output
    }

    pub(in crate::live) fn observe_boundary(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        observed_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let mut output = live_output_from_summary(
            self.summary
                .observe_boundary(trace_id, process, observed_at),
            false,
        );
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
        self.observe_boundary(
            event.envelope.trace_id,
            &event.envelope.process,
            event.envelope.observed_at,
        )
    }

    pub(in crate::live) fn forget_trace(&mut self, trace_id: TraceId) {
        self.enumerate.forget_trace(trace_id);
        self.summary.forget_trace(trace_id);
        self.open_files.retain(|key, _| key.trace_id != trace_id);
        self.linked_file_events
            .retain(|key| key.trace_id != trace_id);
    }

    pub(in crate::live) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let mut output =
            live_output_from_summary(self.summary.finalize_trace(trace_id, finished_at), false);
        append_output(
            &mut output,
            live_output_from_enumerate(self.enumerate.finalize_trace(trace_id, finished_at), false),
        );
        output
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
        LiveSemanticActionOutput {
            actions,
            links: Vec::new(),
            file_observation_paths: Vec::new(),
            file_path_sets: Vec::new(),
            retain_event: true,
            raw_event_consumed: false,
        }
    }

    fn discard_open_state_for_event_fd(&mut self, event: &DomainEvent) {
        let Some(fd) = event_fd(event) else {
            return;
        };
        let key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        };
        self.open_files.remove(&key);
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
            evidence: vec![event_evidence(event, evidence_roles::file::WRITE)],
            attributes: BTreeMap::new(),
        }]
    }
}

fn live_output_from_summary(
    summary: FileSummaryOutput,
    raw_event_consumed: bool,
) -> LiveSemanticActionOutput {
    LiveSemanticActionOutput {
        actions: summary.actions,
        links: Vec::new(),
        file_observation_paths: summary.file_observation_paths,
        file_path_sets: summary.file_path_sets,
        retain_event: summary.retain_event,
        raw_event_consumed,
    }
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

fn event_file_path(event: &DomainEvent) -> Option<String> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .path
        .clone()
        .or_else(|| payload.metadata.get("fd_target").cloned())
}
