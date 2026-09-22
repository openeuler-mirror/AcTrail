use crate::live::actions::{event_action_id, event_evidence};
use model_core::event::{
    DomainEvent, FileIoDirection, FileIoSummary, FilePayload, FileSummaryPathState,
};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};
use std::collections::BTreeMap;

pub(super) struct FileIoAction;

impl FileIoAction {
    pub(super) fn from_summary(
        event: &DomainEvent,
        payload: &FilePayload,
        summary: &FileIoSummary,
        tty: bool,
    ) -> SemanticAction {
        let (kind, count_key, bytes_key) = match (tty, summary.direction) {
            (true, FileIoDirection::Read) => (
                SemanticActionKind::FileTtyIo,
                attrs::file_tty::READ_COUNT,
                attrs::file::BYTES_READ,
            ),
            (true, FileIoDirection::Write) => (
                SemanticActionKind::FileTtyIo,
                attrs::file_tty::WRITE_COUNT,
                attrs::file::BYTES_WRITTEN,
            ),
            (false, FileIoDirection::Read) => (
                SemanticActionKind::FileRead,
                attrs::file::READ_COUNT,
                attrs::file::BYTES_READ,
            ),
            (false, FileIoDirection::Write) => (
                SemanticActionKind::FileWrite,
                attrs::file::WRITE_COUNT,
                attrs::file::BYTES_WRITTEN,
            ),
        };
        let mut attributes = BTreeMap::from([
            ("file.token".to_string(), summary.file_token.to_string()),
            (
                "file.path_observation".to_string(),
                "first_observed".to_string(),
            ),
            (
                "file.interval_basis".to_string(),
                "collector_snapshot".to_string(),
            ),
            ("file.errno".to_string(), summary.errno.to_string()),
            (
                attrs::file::OPERATION.to_string(),
                payload.operation.clone(),
            ),
            (
                "file.path_state".to_string(),
                match summary.path_state {
                    FileSummaryPathState::Resolved => "resolved",
                    FileSummaryPathState::Truncated => "truncated",
                    FileSummaryPathState::Unavailable => "unavailable",
                }
                .to_string(),
            ),
        ]);
        if tty {
            attributes.insert(attrs::file::TTY.to_string(), "true".to_string());
        }
        if let Some(path) = &payload.path {
            attributes.insert(attrs::file::PATH.to_string(), path.clone());
        }
        if let Some(count) = summary.operations {
            attributes.insert(count_key.to_string(), count.to_string());
            if summary.errno != 0 {
                attributes.insert(attrs::file::ERROR_COUNT.to_string(), count.to_string());
            }
        }
        if let Some(bytes) = summary.bytes {
            attributes.insert(bytes_key.to_string(), bytes.to_string());
        }
        SemanticAction {
            action_id: event_action_id(event, kind.as_str()),
            trace_id: event.envelope.trace_id,
            kind,
            title: payload
                .path
                .clone()
                .unwrap_or_else(|| payload.operation.clone()),
            start_time: summary.interval_start,
            end_time: Some(summary.interval_end),
            process: event.envelope.process.clone(),
            status: if summary.errno == 0 {
                SemanticActionStatus::Success
            } else {
                SemanticActionStatus::Error
            },
            completeness: if summary.interval_complete
                && summary.path_state == FileSummaryPathState::Resolved
            {
                SemanticActionCompleteness::Complete
            } else {
                SemanticActionCompleteness::Partial
            },
            attributes,
            evidence: vec![event_evidence(event, "file.io_summary")],
        }
    }
}
