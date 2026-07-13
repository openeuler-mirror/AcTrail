//! Normalization from raw observations into domain event shapes.

use model_core::event::{
    ControlPayload, DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, FilePayload,
    IpcPayload, NetPayload, ProcessPayload, StdioPayload,
};
use model_core::ids::{EventId, TraceId};
use model_core::process::ProcessIdentity;

pub fn normalize_event(
    raw_event: collector_event::RawCollectorEvent,
    trace_id: TraceId,
    process: ProcessIdentity,
    parent: Option<ProcessIdentity>,
    event_id: EventId,
) -> DomainEvent {
    let kind = match &raw_event.payload {
        collector_event::RawObservationPayload::Process { .. } => EventKind::Process,
        collector_event::RawObservationPayload::File { .. } => EventKind::File,
        collector_event::RawObservationPayload::Net { .. } => EventKind::Net,
        collector_event::RawObservationPayload::Ipc { .. } => EventKind::Ipc,
        collector_event::RawObservationPayload::Stdio { .. } => EventKind::Stdio,
    };
    let envelope = EventEnvelope {
        event_id,
        trace_id,
        observed_at: raw_event.envelope.observed_at,
        process,
        collector: raw_event.envelope.collector,
        kind,
        flags: EventFlags::clean(),
    };

    let payload = match raw_event.payload {
        collector_event::RawObservationPayload::Process {
            operation,
            parent: _,
            metadata,
        } => {
            let executable = metadata
                .get("executable")
                .cloned()
                .filter(|value| !value.is_empty());
            EventPayload::Process(ProcessPayload {
                operation,
                parent,
                executable,
                metadata,
            })
        }
        collector_event::RawObservationPayload::File {
            operation,
            path,
            metadata,
        } => EventPayload::File(FilePayload {
            operation,
            path,
            result: metadata
                .get("result")
                .and_then(|value| value.parse::<i32>().ok()),
            metadata,
        }),
        collector_event::RawObservationPayload::Net {
            transport,
            local,
            remote,
            size,
            result,
            metadata,
        } => EventPayload::Net(NetPayload {
            transport,
            local,
            remote,
            size,
            result,
            metadata,
        }),
        collector_event::RawObservationPayload::Ipc {
            channel,
            peer,
            metadata,
        } => EventPayload::Ipc(IpcPayload {
            channel,
            peer,
            size: metadata
                .get("size")
                .and_then(|value| value.parse::<u64>().ok()),
            metadata,
        }),
        collector_event::RawObservationPayload::Stdio {
            stream,
            bytes,
            metadata,
        } => {
            let original_size = metadata
                .get("original_size")
                .and_then(|value| value.parse::<usize>().ok());
            EventPayload::Stdio(StdioPayload {
                stream,
                data: bytes,
                original_size,
                truncated: metadata
                    .get("truncated")
                    .map(|value| value == "true")
                    .unwrap_or(false),
            })
        }
    };

    DomainEvent::new(envelope, payload)
}

#[allow(dead_code)]
pub fn control_event(
    trace_id: TraceId,
    event_id: EventId,
    observed_at: std::time::SystemTime,
    process: model_core::process::ProcessIdentity,
    collector: model_core::ids::CollectorName,
    action: impl Into<String>,
    detail: impl Into<String>,
) -> DomainEvent {
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id,
            observed_at,
            process,
            collector,
            kind: EventKind::Control,
            flags: EventFlags::clean(),
        },
        EventPayload::Control(ControlPayload {
            action: action.into(),
            detail: detail.into(),
        }),
    )
}
