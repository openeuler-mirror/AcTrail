//! File write projection from file syscall events.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkConfidence, SemanticActionLinkRole, SemanticActionStatus,
};

use super::actions::{event_action_id, event_evidence, status_from_result};
use super::runtime::LiveSemanticActionOutput;

pub(super) struct FileWriteProjector {
    open_files: BTreeMap<FileHandleKey, FileWriteState>,
    linked_file_events: BTreeSet<FileEventLinkKey>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileHandleKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    fd: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileWriteState {
    action: Option<SemanticAction>,
    open_event_id: EventId,
    open_time: SystemTime,
    path: String,
    bytes_written: u64,
    write_count: u64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileEventLinkKey {
    trace_id: TraceId,
    parent_action_id: String,
    child_action_id: String,
}

impl FileWriteProjector {
    pub(super) fn new() -> Self {
        Self {
            open_files: BTreeMap::new(),
            linked_file_events: BTreeSet::new(),
        }
    }

    pub(super) fn observe_file_event(
        &mut self,
        event: &DomainEvent,
        file_modify_action: Option<&SemanticAction>,
    ) -> LiveSemanticActionOutput {
        let EventPayload::File(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        match payload.operation.as_str() {
            "open" => {
                self.observe_open(event);
                LiveSemanticActionOutput::default()
            }
            "write" | "writev" => self.observe_write(event, file_modify_action),
            "close" => self.observe_close(event),
            _ => LiveSemanticActionOutput::default(),
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.open_files.retain(|key, _| key.trace_id != trace_id);
        self.linked_file_events
            .retain(|key| key.trace_id != trace_id);
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
            FileWriteState {
                action: None,
                open_event_id: event.envelope.event_id,
                open_time: event.envelope.observed_at,
                path,
                bytes_written: u64::default(),
                write_count: u64::default(),
            },
        );
    }

    fn observe_write(
        &mut self,
        event: &DomainEvent,
        file_modify_action: Option<&SemanticAction>,
    ) -> LiveSemanticActionOutput {
        let Some(path) = event_file_path(event) else {
            return LiveSemanticActionOutput::default();
        };
        let bytes = event_size(event).unwrap_or_default();
        let status = status_from_result(event_result(event));
        let Some(fd) = event_fd(event) else {
            let action = single_write_action(event, &path, bytes, status);
            return LiveSemanticActionOutput {
                actions: vec![action.clone()],
                links: self.file_event_link(&action, file_modify_action, event),
            };
        };
        let key = FileHandleKey {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        };
        let Some(mut state) = self.open_files.remove(&key) else {
            let action = single_write_action(event, &path, bytes, status);
            return LiveSemanticActionOutput {
                actions: vec![action.clone()],
                links: self.file_event_link(&action, file_modify_action, event),
            };
        };
        state.bytes_written = state.bytes_written.saturating_add(bytes);
        state.write_count = state.write_count.saturating_add(1);
        let mut action = state
            .action
            .clone()
            .unwrap_or_else(|| open_backed_write_action(event, &state));
        action.end_time = Some(event.envelope.observed_at);
        action.status = status;
        action.completeness = SemanticActionCompleteness::Partial;
        action.attributes.insert(
            "file.bytes_written".to_string(),
            state.bytes_written.to_string(),
        );
        action.attributes.insert(
            "file.write_count".to_string(),
            state.write_count.to_string(),
        );
        action.evidence.push(event_evidence(event, "file.write"));
        state.action = Some(action.clone());
        self.open_files.insert(key, state);
        LiveSemanticActionOutput {
            actions: vec![action.clone()],
            links: self.file_event_link(&action, file_modify_action, event),
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
        let Some(mut action) = state.action.take() else {
            return LiveSemanticActionOutput::default();
        };
        action.end_time = Some(event.envelope.observed_at);
        action.completeness = SemanticActionCompleteness::Complete;
        action.evidence.push(event_evidence(event, "file.close"));
        LiveSemanticActionOutput {
            actions: vec![action],
            links: Vec::new(),
        }
    }

    fn file_event_link(
        &mut self,
        action: &SemanticAction,
        file_modify_action: Option<&SemanticAction>,
        event: &DomainEvent,
    ) -> Vec<SemanticActionLink> {
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
            evidence: vec![event_evidence(event, "file.write")],
            attributes: BTreeMap::new(),
        }]
    }
}

fn open_backed_write_action(event: &DomainEvent, state: &FileWriteState) -> SemanticAction {
    let mut action = file_write_action(
        format!(
            "trace:{}:event:{}:file.write",
            event.envelope.trace_id.get(),
            state.open_event_id.get()
        ),
        event,
        &state.path,
        state.bytes_written,
        SemanticActionStatus::Success,
    );
    action.start_time = state.open_time;
    action.evidence = vec![semantic_action::SemanticEvidence {
        kind: semantic_action::SemanticEvidenceKind::Event,
        id: state.open_event_id.get(),
        role: "file.open".to_string(),
    }];
    action
}

fn single_write_action(
    event: &DomainEvent,
    path: &str,
    bytes: u64,
    status: SemanticActionStatus,
) -> SemanticAction {
    file_write_action(
        event_action_id(event, "file.write"),
        event,
        path,
        bytes,
        status,
    )
}

fn file_write_action(
    action_id: String,
    event: &DomainEvent,
    path: &str,
    bytes: u64,
    status: SemanticActionStatus,
) -> SemanticAction {
    let mut attributes = BTreeMap::from([
        ("file.path".to_string(), path.to_string()),
        ("file.bytes_written".to_string(), bytes.to_string()),
    ]);
    if let Some(fd) = event_fd(event) {
        attributes.insert("file.fd".to_string(), fd.to_string());
    }
    SemanticAction {
        action_id,
        trace_id: event.envelope.trace_id,
        kind: SemanticActionKind::FileWrite,
        title: path.to_string(),
        start_time: event.envelope.observed_at,
        end_time: Some(event.envelope.observed_at),
        process: event.envelope.process.clone(),
        status,
        completeness: SemanticActionCompleteness::Partial,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(event, "file.write")],
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

fn event_fd(event: &DomainEvent) -> Option<u32> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .metadata
        .get("fd")
        .and_then(|value| value.parse::<u32>().ok())
}

fn event_result(event: &DomainEvent) -> Option<i32> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload.result
}

fn event_size(event: &DomainEvent) -> Option<u64> {
    let EventPayload::File(payload) = &event.payload else {
        return None;
    };
    payload
        .metadata
        .get("size")
        .and_then(|value| value.parse::<u64>().ok())
}
