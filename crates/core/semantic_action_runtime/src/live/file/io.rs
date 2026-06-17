use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::event::DomainEvent;
use model_core::ids::EventId;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs, evidence_roles,
};

use super::common::event_fd;
use crate::live::actions::{event_action_id, event_evidence};

#[derive(Clone, Copy)]
pub(super) enum FileAccessKind {
    Read,
    Write,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct FileIoState {
    pub(super) action: Option<SemanticAction>,
    pub(super) bytes: u64,
    pub(super) count: u64,
}

impl FileAccessKind {
    const fn action_kind(self) -> SemanticActionKind {
        match self {
            Self::Read => SemanticActionKind::FileRead,
            Self::Write => SemanticActionKind::FileWrite,
        }
    }

    const fn action_suffix(self) -> &'static str {
        self.action_kind().as_str()
    }

    pub(super) const fn bytes_attr(self) -> &'static str {
        match self {
            Self::Read => attrs::file::BYTES_READ,
            Self::Write => attrs::file::BYTES_WRITTEN,
        }
    }

    pub(super) const fn count_attr(self) -> &'static str {
        match self {
            Self::Read => attrs::file::READ_COUNT,
            Self::Write => attrs::file::WRITE_COUNT,
        }
    }

    pub(super) const fn event_role(self) -> &'static str {
        self.action_kind().as_str()
    }
}

pub(super) fn open_backed_io_action(
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
        role: evidence_roles::file::OPEN.to_string(),
    }];
    action
}

pub(super) fn single_io_action(
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

pub(super) fn complete_close_action(
    io: &mut FileIoState,
    event: &DomainEvent,
) -> Option<SemanticAction> {
    let mut action = io.action.take()?;
    action.end_time = Some(event.envelope.observed_at);
    action.completeness = SemanticActionCompleteness::Complete;
    action
        .evidence
        .push(event_evidence(event, evidence_roles::file::CLOSE));
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
        (attrs::file::PATH.to_string(), path.to_string()),
        (kind.bytes_attr().to_string(), bytes.to_string()),
    ]);
    if let Some(fd) = event_fd(event) {
        attributes.insert(attrs::file::FD.to_string(), fd.to_string());
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
