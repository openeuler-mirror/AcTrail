//! File access projection from file syscall events.

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

pub(super) struct FileAccessProjector {
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FileIoState {
    action: Option<SemanticAction>,
    bytes: u64,
    count: u64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileEventLinkKey {
    trace_id: TraceId,
    parent_action_id: String,
    child_action_id: String,
}

#[derive(Clone, Copy)]
enum FileAccessKind {
    Read,
    Write,
}

impl FileAccessProjector {
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
            "read" | "readv" => self.observe_io(event, FileAccessKind::Read, None),
            "write" | "writev" => self.observe_io(event, FileAccessKind::Write, file_modify_action),
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
            evidence: vec![event_evidence(event, "file.write")],
            attributes: BTreeMap::new(),
        }]
    }
}

impl FileHandleState {
    fn io_mut(&mut self, kind: FileAccessKind) -> &mut FileIoState {
        match kind {
            FileAccessKind::Read => &mut self.read,
            FileAccessKind::Write => &mut self.write,
        }
    }
}

impl FileAccessKind {
    const fn action_kind(self) -> SemanticActionKind {
        match self {
            Self::Read => SemanticActionKind::FileRead,
            Self::Write => SemanticActionKind::FileWrite,
        }
    }

    const fn action_suffix(self) -> &'static str {
        match self {
            Self::Read => "file.read",
            Self::Write => "file.write",
        }
    }

    const fn bytes_attr(self) -> &'static str {
        match self {
            Self::Read => "file.bytes_read",
            Self::Write => "file.bytes_written",
        }
    }

    const fn count_attr(self) -> &'static str {
        match self {
            Self::Read => "file.read_count",
            Self::Write => "file.write_count",
        }
    }

    const fn event_role(self) -> &'static str {
        match self {
            Self::Read => "file.read",
            Self::Write => "file.write",
        }
    }
}

fn open_backed_io_action(
    event: &DomainEvent,
    kind: FileAccessKind,
    open_event_id: EventId,
    open_time: SystemTime,
    path: &str,
) -> SemanticAction {
    let mut action = file_io_action(
        format!(
            "trace:{}:event:{}:{}",
            event.envelope.trace_id.get(),
            open_event_id.get(),
            kind.action_suffix()
        ),
        event,
        kind,
        path,
        0,
        SemanticActionStatus::Success,
    );
    action.start_time = open_time;
    action.evidence = vec![semantic_action::SemanticEvidence {
        kind: semantic_action::SemanticEvidenceKind::Event,
        id: open_event_id.get(),
        role: "file.open".to_string(),
    }];
    action
}

fn single_io_action(
    event: &DomainEvent,
    kind: FileAccessKind,
    path: &str,
    bytes: u64,
    status: SemanticActionStatus,
) -> SemanticAction {
    file_io_action(
        event_action_id(event, kind.action_suffix()),
        event,
        kind,
        path,
        bytes,
        status,
    )
}

fn complete_close_action(io: &mut FileIoState, event: &DomainEvent) -> Option<SemanticAction> {
    let mut action = io.action.take()?;
    action.end_time = Some(event.envelope.observed_at);
    action.completeness = SemanticActionCompleteness::Complete;
    action.evidence.push(event_evidence(event, "file.close"));
    Some(action)
}

fn file_io_action(
    action_id: String,
    event: &DomainEvent,
    kind: FileAccessKind,
    path: &str,
    bytes: u64,
    status: SemanticActionStatus,
) -> SemanticAction {
    let mut attributes = BTreeMap::from([
        ("file.path".to_string(), path.to_string()),
        (kind.bytes_attr().to_string(), bytes.to_string()),
    ]);
    if let Some(fd) = event_fd(event) {
        attributes.insert("file.fd".to_string(), fd.to_string());
    }
    SemanticAction {
        action_id,
        trace_id: event.envelope.trace_id,
        kind: kind.action_kind(),
        title: path.to_string(),
        start_time: event.envelope.observed_at,
        end_time: Some(event.envelope.observed_at),
        process: event.envelope.process.clone(),
        status,
        completeness: SemanticActionCompleteness::Partial,
        confidence_millis: None,
        attributes,
        evidence: vec![event_evidence(event, kind.event_role())],
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
