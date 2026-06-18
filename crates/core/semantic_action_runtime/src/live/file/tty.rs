use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::event::DomainEvent;
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionStatus,
    attr_keys as attrs,
};

use super::common::{event_result, event_size};
use crate::live::actions::event_action_id;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct TtyKey {
    pub(super) trace_id: TraceId,
    pub(super) process: ProcessIdentity,
    pub(super) path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TtyState {
    action_id: String,
    trace_id: TraceId,
    process: ProcessIdentity,
    path: String,
    start_time: SystemTime,
    first_event_id: EventId,
    last_event_id: EventId,
    open_count: u64,
    close_count: u64,
    read_count: u64,
    write_count: u64,
    bytes_read: u64,
    bytes_written: u64,
    error_count: u64,
}

impl TtyState {
    pub(super) fn new(event: &DomainEvent, path: &str) -> Self {
        Self {
            action_id: event_action_id(event, SemanticActionKind::FileTtyIo.as_str()),
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            path: path.to_string(),
            start_time: event.envelope.observed_at,
            first_event_id: event.envelope.event_id,
            last_event_id: event.envelope.event_id,
            open_count: 0,
            close_count: 0,
            read_count: 0,
            write_count: 0,
            bytes_read: 0,
            bytes_written: 0,
            error_count: 0,
        }
    }

    pub(super) fn observe(&mut self, event: &DomainEvent, operation: &str) {
        self.last_event_id = event.envelope.event_id;
        if event_result(event).is_some_and(|result| result < 0) {
            self.error_count = self.error_count.saturating_add(1);
        }
        match operation {
            "open" => self.open_count = self.open_count.saturating_add(1),
            "close" => self.close_count = self.close_count.saturating_add(1),
            "read" | "readv" => {
                self.read_count = self.read_count.saturating_add(1);
                self.bytes_read = self
                    .bytes_read
                    .saturating_add(event_size(event).unwrap_or(0));
            }
            "write" | "writev" => {
                self.write_count = self.write_count.saturating_add(1);
                self.bytes_written = self
                    .bytes_written
                    .saturating_add(event_size(event).unwrap_or(0));
            }
            _ => {}
        }
    }

    pub(super) fn action(
        &self,
        end_time: SystemTime,
        completeness: SemanticActionCompleteness,
    ) -> SemanticAction {
        let mut attributes = BTreeMap::from([
            (attrs::file::PATH.to_string(), self.path.clone()),
            (attrs::file::TTY.to_string(), "true".to_string()),
            (
                attrs::file_tty::OPEN_COUNT.to_string(),
                self.open_count.to_string(),
            ),
            (
                attrs::file_tty::CLOSE_COUNT.to_string(),
                self.close_count.to_string(),
            ),
            (
                attrs::file_tty::READ_COUNT.to_string(),
                self.read_count.to_string(),
            ),
            (
                attrs::file_tty::WRITE_COUNT.to_string(),
                self.write_count.to_string(),
            ),
            (
                attrs::file::BYTES_READ.to_string(),
                self.bytes_read.to_string(),
            ),
            (
                attrs::file::BYTES_WRITTEN.to_string(),
                self.bytes_written.to_string(),
            ),
            (
                attrs::file_tty::ERROR_COUNT.to_string(),
                self.error_count.to_string(),
            ),
            (
                attrs::file_tty::FIRST_EVENT_ID.to_string(),
                self.first_event_id.get().to_string(),
            ),
            (
                attrs::file_tty::LAST_EVENT_ID.to_string(),
                self.last_event_id.get().to_string(),
            ),
        ]);
        attributes.insert(
            attrs::file_tty::EVENT_COUNT.to_string(),
            self.event_count().to_string(),
        );
        SemanticAction {
            action_id: self.action_id.clone(),
            trace_id: self.trace_id,
            kind: SemanticActionKind::FileTtyIo,
            title: self.path.clone(),
            start_time: self.start_time,
            end_time: Some(end_time),
            process: self.process.clone(),
            status: aggregate_status(self.error_count),
            completeness,
            confidence_millis: None,
            attributes,
            evidence: Vec::new(),
        }
    }

    fn event_count(&self) -> u64 {
        self.open_count
            .saturating_add(self.close_count)
            .saturating_add(self.read_count)
            .saturating_add(self.write_count)
    }
}

fn aggregate_status(error_count: u64) -> SemanticActionStatus {
    if error_count == 0 {
        SemanticActionStatus::Success
    } else {
        SemanticActionStatus::Error
    }
}
