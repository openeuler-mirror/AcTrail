use std::collections::BTreeMap;
use std::time::SystemTime;

use config_core::daemon::{
    AgentInvocationConfig, FileObservationConfig, PayloadMcpConfig, SemanticRetentionConfig,
};
use model_core::event::{
    ApplicationPayload, DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload,
    FilePayload, ProcessPayload,
};
use model_core::ids::{CollectorName, EventId, TraceId};
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadRedactionState,
    PayloadSegment, PayloadSegmentId, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;

use super::LiveSemanticActionRuntime;

pub(super) const TRACE_ID: TraceId = TraceId::new(42);
pub(super) const OTHER_TRACE_ID: TraceId = TraceId::new(43);
pub(super) const ROOT_PID: u32 = 1000;
pub(super) const WRAPPER_PID: u32 = 1001;
pub(super) const AGENT_PID: u32 = 1002;
pub(super) const ROOT_GENERATION: u64 = 7000;
pub(super) const WRAPPER_GENERATION: u64 = 7001;
pub(super) const AGENT_GENERATION: u64 = 7002;
pub(super) const ROOT_START_TICKS: u64 = 5000;
pub(super) const WRAPPER_START_TICKS: u64 = 5001;
pub(super) const AGENT_START_TICKS: u64 = 5002;
pub(super) const ROOT_EXEC_EVENT_ID: EventId = EventId::new(10);
pub(super) const WRAPPER_EXEC_EVENT_ID: EventId = EventId::new(11);
pub(super) const AGENT_EXEC_EVENT_ID: EventId = EventId::new(12);
pub(super) const ROOT_EXIT_EVENT_ID: EventId = EventId::new(13);
pub(super) const ROOT_LATE_EXEC_EVENT_ID: EventId = EventId::new(14);
pub(super) const HTTP_REQUEST_EVENT_ID: EventId = EventId::new(15);
pub(super) const HTTP_RESPONSE_EVENT_ID: EventId = EventId::new(16);
pub(super) const FILE_OPEN_EVENT_ID: EventId = EventId::new(17);
pub(super) const FILE_READ_EVENT_ID: EventId = EventId::new(18);
pub(super) const FILE_CLOSE_EVENT_ID: EventId = EventId::new(19);
pub(super) const FORK_ATTEMPT_EVENT_ID: EventId = EventId::new(20);
pub(super) const HTTP_CONNECT_EVENT_ID: EventId = EventId::new(21);
pub(super) const HTTP_CONNECT_RESPONSE_EVENT_ID: EventId = EventId::new(22);
pub(super) const AGENT_FORK_EVENT_ID: EventId = EventId::new(23);
pub(super) const AGENT_DUPLICATE_EXEC_EVENT_ID: EventId = EventId::new(24);
pub(super) const AGENT_SECOND_FORK_EVENT_ID: EventId = EventId::new(25);
pub(super) const PAYLOAD_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(30);
pub(super) const RESPONSE_FIRST_SEGMENT_ID: PayloadSegmentId = PayloadSegmentId::new(40);
pub(super) const PAYLOAD_OPERATION_ID: u64 = 31;
pub(super) const RESPONSE_FIRST_OPERATION_ID: u64 = 41;
pub(super) const PAYLOAD_SEQUENCE: u64 = 0;
pub(super) const RESPONSE_FIRST_SEQUENCE: u64 = 1;
pub(super) const PAYLOAD_OFFSET: u64 = 0;
pub(super) const SUCCESS_EXIT_CODE: i32 = 0;
pub(super) const AGENT_ACTION_SEQUENCE_ATTR: &str = "agent.performed_action.sequence";
pub(super) const FIRST_AGENT_ACTION_SEQUENCE: &str = "0";
pub(super) const SECOND_AGENT_ACTION_SEQUENCE: &str = "1";
pub(super) const TEST_FILE_FD: u32 = 7;
pub(super) const TEST_FILE_PATH: &str = "/root/.config/xiaoo/config.json";
pub(super) const TEST_FILE_READ_BYTES: u64 = 128;
const TEST_CONNECT_HOST: &str = "api.local";
const TEST_CONNECT_AUTHORITY: &str = "api.local:443";
const TEST_CONNECT_STREAM_KEY: &str = "socket-1";
const TEST_CONNECT_REQUEST_SEGMENT_ID: &str = "100";
const TEST_CONNECT_RESPONSE_SEGMENT_ID: &str = "101";

pub(super) fn runtime() -> LiveSemanticActionRuntime {
    LiveSemanticActionRuntime::new(
        AgentInvocationConfig {
            enabled: true,
            commands: vec!["xiaoo".to_string()],
        },
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        PayloadMcpConfig::default(),
    )
}

pub(super) fn runtime_with_mcp_parse_buffer_max_bytes(
    parse_buffer_max_bytes: u64,
) -> LiveSemanticActionRuntime {
    LiveSemanticActionRuntime::new(
        AgentInvocationConfig {
            enabled: true,
            commands: vec!["xiaoo".to_string()],
        },
        SemanticRetentionConfig::default(),
        FileObservationConfig::default(),
        PayloadMcpConfig {
            parse_buffer_max_bytes,
        },
    )
}

pub(super) fn exec_event(
    event_id: EventId,
    process: ProcessIdentity,
    parent: Option<ProcessIdentity>,
    executable: &str,
) -> DomainEvent {
    let mut metadata = BTreeMap::new();
    if let Some(parent) = &parent {
        metadata.insert("ppid".to_string(), parent.pid.to_string());
    }
    metadata.insert("command_line".to_string(), executable.to_string());
    process_event(
        event_id,
        process,
        ProcessPayload {
            operation: "exec".to_string(),
            parent,
            executable: Some(executable.to_string()),
            metadata,
        },
    )
}

pub(super) fn exit_event(
    event_id: EventId,
    process: ProcessIdentity,
    exit_code: i32,
) -> DomainEvent {
    let mut metadata = BTreeMap::new();
    metadata.insert("exit_code".to_string(), exit_code.to_string());
    process_event(
        event_id,
        process,
        ProcessPayload {
            operation: "exit".to_string(),
            parent: None,
            executable: None,
            metadata,
        },
    )
}

pub(super) fn fork_attempt_event(event_id: EventId, process: ProcessIdentity) -> DomainEvent {
    let metadata = BTreeMap::from([
        ("process.operation".to_string(), "fork_attempt".to_string()),
        ("syscall".to_string(), "clone3".to_string()),
    ]);
    process_event(
        event_id,
        process,
        ProcessPayload {
            operation: "fork_attempt".to_string(),
            parent: None,
            executable: None,
            metadata,
        },
    )
}

pub(super) fn fork_event(
    event_id: EventId,
    process: ProcessIdentity,
    parent: ProcessIdentity,
) -> DomainEvent {
    let metadata = BTreeMap::from([
        ("ppid".to_string(), parent.pid.to_string()),
        ("stat_ppid".to_string(), parent.pid.to_string()),
    ]);
    process_event(
        event_id,
        process,
        ProcessPayload {
            operation: "fork".to_string(),
            parent: Some(parent),
            executable: None,
            metadata,
        },
    )
}

pub(super) fn llm_payload_segment(process: ProcessIdentity) -> PayloadSegment {
    let body = r#"{"model":"deepseek-chat","messages":[{"role":"user","content":"hello"}]}"#;
    let bytes = format!(
        "POST /chat/completions HTTP/1.1\r\nHost: {TEST_CONNECT_HOST}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    payload_segment(
        process,
        PayloadDirection::Outbound,
        PAYLOAD_SEGMENT_ID,
        PAYLOAD_OPERATION_ID,
        PAYLOAD_SEQUENCE,
        bytes,
    )
}

pub(super) fn http_request_event(event_id: EventId, process: ProcessIdentity) -> DomainEvent {
    let metadata = BTreeMap::from([
        ("direction".to_string(), "outbound".to_string()),
        ("source_boundary".to_string(), "TlsUserSpace".to_string()),
        ("stream_key".to_string(), "stream-1".to_string()),
        ("payload_sequence".to_string(), PAYLOAD_SEQUENCE.to_string()),
        (
            "payload_segment_id".to_string(),
            PAYLOAD_SEGMENT_ID.get().to_string(),
        ),
        ("method".to_string(), "POST".to_string()),
        ("target".to_string(), "/chat/completions".to_string()),
    ]);
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: observed_at(),
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::Application,
            flags: EventFlags::clean(),
        },
        EventPayload::Application(ApplicationPayload {
            protocol: "HTTP/1.1".to_string(),
            operation: "request".to_string(),
            summary: "POST /chat/completions".to_string(),
            metadata,
        }),
    )
}

pub(super) fn http_response_event(event_id: EventId, process: ProcessIdentity) -> DomainEvent {
    let metadata = BTreeMap::from([
        ("direction".to_string(), "inbound".to_string()),
        ("source_boundary".to_string(), "TlsUserSpace".to_string()),
        ("stream_key".to_string(), "stream-1".to_string()),
        (
            "payload_sequence".to_string(),
            RESPONSE_FIRST_SEQUENCE.to_string(),
        ),
        (
            "payload_segment_id".to_string(),
            RESPONSE_FIRST_SEGMENT_ID.get().to_string(),
        ),
        ("status_code".to_string(), "200".to_string()),
        ("reason".to_string(), "OK".to_string()),
    ]);
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: observed_at(),
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::Application,
            flags: EventFlags::clean(),
        },
        EventPayload::Application(ApplicationPayload {
            protocol: "HTTP/1.1".to_string(),
            operation: "response".to_string(),
            summary: "200 OK".to_string(),
            metadata,
        }),
    )
}

pub(super) fn http_connect_event(event_id: EventId, process: ProcessIdentity) -> DomainEvent {
    let metadata = BTreeMap::from([
        ("direction".to_string(), "outbound".to_string()),
        ("source_boundary".to_string(), "Syscall".to_string()),
        (
            "stream_key".to_string(),
            TEST_CONNECT_STREAM_KEY.to_string(),
        ),
        ("payload_sequence".to_string(), PAYLOAD_SEQUENCE.to_string()),
        (
            "payload_segment_id".to_string(),
            TEST_CONNECT_REQUEST_SEGMENT_ID.to_string(),
        ),
        ("method".to_string(), "CONNECT".to_string()),
        ("target".to_string(), TEST_CONNECT_AUTHORITY.to_string()),
        ("host".to_string(), TEST_CONNECT_AUTHORITY.to_string()),
    ]);
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: observed_at(),
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::Application,
            flags: EventFlags::clean(),
        },
        EventPayload::Application(ApplicationPayload {
            protocol: "HTTP/1.1".to_string(),
            operation: "request".to_string(),
            summary: format!("CONNECT {TEST_CONNECT_AUTHORITY}"),
            metadata,
        }),
    )
}

pub(super) fn http_connect_response_event(
    event_id: EventId,
    process: ProcessIdentity,
) -> DomainEvent {
    let metadata = BTreeMap::from([
        ("direction".to_string(), "inbound".to_string()),
        ("source_boundary".to_string(), "Syscall".to_string()),
        (
            "stream_key".to_string(),
            TEST_CONNECT_STREAM_KEY.to_string(),
        ),
        (
            "payload_sequence".to_string(),
            RESPONSE_FIRST_SEQUENCE.to_string(),
        ),
        (
            "payload_segment_id".to_string(),
            TEST_CONNECT_RESPONSE_SEGMENT_ID.to_string(),
        ),
        ("status_code".to_string(), "200".to_string()),
        ("reason".to_string(), "Connection established".to_string()),
    ]);
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: observed_at(),
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::Application,
            flags: EventFlags::clean(),
        },
        EventPayload::Application(ApplicationPayload {
            protocol: "HTTP/1.1".to_string(),
            operation: "response".to_string(),
            summary: "200 Connection established".to_string(),
            metadata,
        }),
    )
}

pub(super) fn file_event(
    event_id: EventId,
    process: ProcessIdentity,
    operation: &str,
    result: i32,
    size: Option<u64>,
) -> DomainEvent {
    let mut metadata = BTreeMap::from([
        ("operation".to_string(), operation.to_string()),
        ("result".to_string(), result.to_string()),
        ("fd".to_string(), TEST_FILE_FD.to_string()),
        ("fd_target".to_string(), TEST_FILE_PATH.to_string()),
    ]);
    if let Some(size) = size {
        metadata.insert("size".to_string(), size.to_string());
    }
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: observed_at(),
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::File,
            flags: EventFlags::clean(),
        },
        EventPayload::File(FilePayload {
            operation: operation.to_string(),
            path: Some(TEST_FILE_PATH.to_string()),
            result: Some(result),
            metadata,
        }),
    )
}

pub(super) fn llm_response_payload_segment(
    process: ProcessIdentity,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    bytes: Vec<u8>,
) -> PayloadSegment {
    payload_segment(
        process,
        PayloadDirection::Inbound,
        segment_id,
        operation_id,
        sequence,
        bytes,
    )
}

pub(super) fn outbound_http1_payload_segment_with_bytes(
    process: ProcessIdentity,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    bytes: Vec<u8>,
) -> PayloadSegment {
    payload_segment(
        process,
        PayloadDirection::Outbound,
        segment_id,
        operation_id,
        sequence,
        bytes,
    )
}

pub(super) fn stdio_payload_segment_with_bytes(
    process: ProcessIdentity,
    direction: PayloadDirection,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    stream_key: &str,
    bytes: Vec<u8>,
) -> PayloadSegment {
    let size = bytes.len() as u64;
    PayloadSegment {
        segment_id,
        trace_id: TRACE_ID,
        observed_at: observed_at(),
        process,
        source_boundary: PayloadSourceBoundary::Stdio,
        content_state: PayloadContentState::Plaintext,
        direction,
        stream_key: PayloadStreamKey::new(stream_key),
        sequence,
        original_size: size,
        captured_size: size,
        operation_id,
        operation_offset: PAYLOAD_OFFSET,
        operation_original_size: size,
        operation_captured_size: size,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        redaction: PayloadRedactionState::Unredacted,
        library: "stdio".to_string(),
        symbol: "stdio_payload".to_string(),
        protocol_hint: None,
        bytes,
    }
}

pub(super) fn response_segment_id(index: usize) -> PayloadSegmentId {
    PayloadSegmentId::new(RESPONSE_FIRST_SEGMENT_ID.get() + index as u64)
}

pub(super) fn response_operation_id(index: usize) -> u64 {
    RESPONSE_FIRST_OPERATION_ID + index as u64
}

pub(super) fn response_sequence(index: usize) -> u64 {
    RESPONSE_FIRST_SEQUENCE + index as u64
}

pub(super) fn http_chunk_prefix(body: &str) -> Vec<u8> {
    format!("{:x}\r\n", body.len()).into_bytes()
}

fn process_event(
    event_id: EventId,
    process: ProcessIdentity,
    payload: ProcessPayload,
) -> DomainEvent {
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: observed_at(),
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::Process,
            flags: EventFlags::clean(),
        },
        EventPayload::Process(payload),
    )
}

fn payload_segment(
    process: ProcessIdentity,
    direction: PayloadDirection,
    segment_id: PayloadSegmentId,
    operation_id: u64,
    sequence: u64,
    bytes: Vec<u8>,
) -> PayloadSegment {
    let size = bytes.len() as u64;
    PayloadSegment {
        segment_id,
        trace_id: TRACE_ID,
        observed_at: observed_at(),
        process,
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction,
        stream_key: PayloadStreamKey::new("stream-1"),
        sequence,
        original_size: size,
        captured_size: size,
        operation_id,
        operation_offset: PAYLOAD_OFFSET,
        operation_original_size: size,
        operation_captured_size: size,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        redaction: PayloadRedactionState::Unredacted,
        library: "rustls".to_string(),
        symbol: "buffer_plaintext".to_string(),
        protocol_hint: Some("http/1.x".to_string()),
        bytes,
    }
}

fn observed_at() -> SystemTime {
    SystemTime::UNIX_EPOCH
}
