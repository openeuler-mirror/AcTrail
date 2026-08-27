use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::event::DomainEvent;
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    SemanticEvidence, SemanticEvidenceKind, attr_keys as attrs, evidence_roles,
};

use super::super::shared::event_fd;
use crate::live::actions::{event_action_id, event_action_id_for_event_id, event_evidence};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FileAccessKind {
    Read,
    Write,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct FileIoState {
    first_event_id: Option<EventId>,
    last_event_id: Option<EventId>,
    bytes: u64,
    count: u64,
    error_count: u64,
}

impl FileIoState {
    pub(super) fn observe(&mut self, event_id: EventId, bytes: u64, count: u64, error_count: u64) {
        self.first_event_id.get_or_insert(event_id);
        self.last_event_id = Some(event_id);
        self.bytes = self.bytes.saturating_add(bytes);
        self.count = self.count.saturating_add(count);
        self.error_count = self.error_count.saturating_add(error_count);
    }

    fn status(&self) -> SemanticActionStatus {
        if self.error_count == 0 {
            SemanticActionStatus::Success
        } else {
            SemanticActionStatus::Error
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
        self.action_kind().as_str()
    }

    const fn bytes_attr(self) -> &'static str {
        match self {
            Self::Read => attrs::file::BYTES_READ,
            Self::Write => attrs::file::BYTES_WRITTEN,
        }
    }

    const fn count_attr(self) -> &'static str {
        match self {
            Self::Read => attrs::file::READ_COUNT,
            Self::Write => attrs::file::WRITE_COUNT,
        }
    }

    const fn event_role(self) -> &'static str {
        self.action_kind().as_str()
    }
}

pub(super) struct OpenFileActionContext<'a> {
    pub(super) trace_id: TraceId,
    pub(super) process: &'a ProcessIdentity,
    pub(super) fd: u32,
    pub(super) action_seed_event_id: EventId,
    pub(super) open_event_id: EventId,
    pub(super) open_evidence_role: Option<&'static str>,
    pub(super) open_time: SystemTime,
    pub(super) path: &'a str,
}

pub(super) fn terminal_io_action(
    context: OpenFileActionContext<'_>,
    kind: FileAccessKind,
    io: &FileIoState,
    end_time: SystemTime,
    completeness: SemanticActionCompleteness,
    close_event_id: Option<EventId>,
) -> Option<SemanticAction> {
    let first_event_id = io.first_event_id?;
    let attributes = BTreeMap::from([
        (attrs::file::PATH.to_string(), context.path.to_string()),
        (attrs::file::FD.to_string(), context.fd.to_string()),
        (kind.bytes_attr().to_string(), io.bytes.to_string()),
        (kind.count_attr().to_string(), io.count.to_string()),
        (
            attrs::file::ERROR_COUNT.to_string(),
            io.error_count.to_string(),
        ),
    ]);

    let mut evidence = Vec::with_capacity(4);
    if let Some(open_evidence_role) = context.open_evidence_role {
        evidence.push(event_id_evidence(context.open_event_id, open_evidence_role));
    }
    evidence.push(event_id_evidence(first_event_id, kind.event_role()));
    if let Some(last_event_id) = io
        .last_event_id
        .filter(|last_event_id| *last_event_id != first_event_id)
    {
        evidence.push(event_id_evidence(last_event_id, kind.event_role()));
    }
    if let Some(close_event_id) = close_event_id {
        evidence.push(event_id_evidence(
            close_event_id,
            evidence_roles::file::CLOSE,
        ));
    }

    Some(SemanticAction {
        action_id: event_action_id_for_event_id(
            context.trace_id,
            context.action_seed_event_id,
            kind.action_suffix(),
        ),
        trace_id: context.trace_id,
        kind: kind.action_kind(),
        title: context.path.to_string(),
        start_time: context.open_time,
        end_time: Some(end_time),
        process: context.process.clone(),
        status: io.status(),
        completeness,
        attributes,
        evidence,
    })
}

pub(super) fn single_io_action(
    event: &DomainEvent,
    kind: FileAccessKind,
    path: &str,
    bytes: u64,
    count: u64,
    error_count: u64,
    status: SemanticActionStatus,
) -> SemanticAction {
    let status = if error_count > 0 {
        SemanticActionStatus::Error
    } else {
        status
    };
    let mut action = file_io_action(
        event_action_id(event, kind.action_suffix()),
        event,
        kind,
        path,
        bytes,
        count,
        error_count,
        status,
    );
    action.completeness = SemanticActionCompleteness::Complete;
    action
}

fn file_io_action(
    action_id: String,
    event: &DomainEvent,
    kind: FileAccessKind,
    path: &str,
    bytes: u64,
    count: u64,
    error_count: u64,
    status: SemanticActionStatus,
) -> SemanticAction {
    let mut attributes = BTreeMap::from([
        (attrs::file::PATH.to_string(), path.to_string()),
        (kind.bytes_attr().to_string(), bytes.to_string()),
        (kind.count_attr().to_string(), count.to_string()),
        (
            attrs::file::ERROR_COUNT.to_string(),
            error_count.to_string(),
        ),
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
        completeness: SemanticActionCompleteness::Complete,
        attributes,
        evidence: vec![event_evidence(event, kind.event_role())],
    }
}

fn event_id_evidence(event_id: EventId, role: &str) -> SemanticEvidence {
    SemanticEvidence {
        kind: SemanticEvidenceKind::Event,
        id: event_id.get(),
        role: role.to_string(),
    }
}
