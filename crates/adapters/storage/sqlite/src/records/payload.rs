//! Event-payload encoding used by the SQLite storage adapter.

use std::collections::BTreeMap;

use model_core::event::{
    ApplicationPayload, ControlPayload, EnforcementPayload, EventPayload, FilePayload, IpcPayload,
    LabelPayload, LossPayload, NetPayload, ProcessPayload, ResourcePayload, StdioPayload,
};
use model_core::process::{NamespaceIdentity, ProcessIdentity};
use rusqlite::Error as SqlError;

use crate::records::helpers::{
    decode_bytes, decode_map, encode_bytes, encode_map, escape, unescape,
};

pub fn encode_event_payload(payload: &EventPayload) -> (String, String, String) {
    match payload {
        EventPayload::Process(payload) => (
            "process".to_string(),
            encode_map(&BTreeMap::from([
                ("operation".to_string(), payload.operation.clone()),
                (
                    "parent".to_string(),
                    payload
                        .parent
                        .as_ref()
                        .map(encode_process_identity_inline)
                        .unwrap_or_default(),
                ),
                (
                    "executable".to_string(),
                    payload.executable.clone().unwrap_or_default(),
                ),
                ("metadata".to_string(), encode_map(&payload.metadata)),
            ])),
            String::new(),
        ),
        EventPayload::File(payload) => (
            "file".to_string(),
            encode_map(&BTreeMap::from([
                ("operation".to_string(), payload.operation.clone()),
                ("path".to_string(), payload.path.clone().unwrap_or_default()),
                (
                    "result".to_string(),
                    payload
                        .result
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                ("metadata".to_string(), encode_map(&payload.metadata)),
            ])),
            String::new(),
        ),
        EventPayload::Net(payload) => (
            "net".to_string(),
            encode_map(&BTreeMap::from([
                ("transport".to_string(), payload.transport.clone()),
                (
                    "local".to_string(),
                    payload.local.clone().unwrap_or_default(),
                ),
                (
                    "remote".to_string(),
                    payload.remote.clone().unwrap_or_default(),
                ),
                (
                    "size".to_string(),
                    payload
                        .size
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                (
                    "result".to_string(),
                    payload
                        .result
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                ("metadata".to_string(), encode_map(&payload.metadata)),
            ])),
            String::new(),
        ),
        EventPayload::Ipc(payload) => (
            "ipc".to_string(),
            encode_map(&BTreeMap::from([
                ("channel".to_string(), payload.channel.clone()),
                ("peer".to_string(), payload.peer.clone().unwrap_or_default()),
                (
                    "size".to_string(),
                    payload
                        .size
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                ("metadata".to_string(), encode_map(&payload.metadata)),
            ])),
            String::new(),
        ),
        EventPayload::Stdio(payload) => (
            "stdio".to_string(),
            encode_map(&BTreeMap::from([
                ("stream".to_string(), payload.stream.clone()),
                (
                    "original_size".to_string(),
                    payload
                        .original_size
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                ("truncated".to_string(), payload.truncated.to_string()),
            ])),
            encode_bytes(&payload.data),
        ),
        EventPayload::Application(payload) => (
            "application".to_string(),
            encode_map(&BTreeMap::from([
                ("protocol".to_string(), payload.protocol.clone()),
                ("operation".to_string(), payload.operation.clone()),
                ("summary".to_string(), payload.summary.clone()),
                ("metadata".to_string(), encode_map(&payload.metadata)),
            ])),
            String::new(),
        ),
        EventPayload::Resource(payload) => (
            "resource".to_string(),
            encode_map(&BTreeMap::from([
                ("scope".to_string(), payload.scope.clone()),
                ("subject".to_string(), payload.subject.clone()),
                (
                    "cpu_percent_millis".to_string(),
                    payload
                        .cpu_percent_millis
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                (
                    "rss_kb".to_string(),
                    payload
                        .rss_kb
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                (
                    "virtual_memory_kb".to_string(),
                    payload
                        .virtual_memory_kb
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                ("metadata".to_string(), encode_map(&payload.metadata)),
            ])),
            String::new(),
        ),
        EventPayload::Control(payload) => (
            "control".to_string(),
            encode_map(&BTreeMap::from([
                ("action".to_string(), payload.action.clone()),
                ("detail".to_string(), payload.detail.clone()),
            ])),
            String::new(),
        ),
        EventPayload::Loss(payload) => (
            "loss".to_string(),
            encode_map(&BTreeMap::from([
                ("reason".to_string(), payload.reason.clone()),
                ("fatal".to_string(), payload.fatal.to_string()),
            ])),
            String::new(),
        ),
        EventPayload::Label(payload) => (
            "label".to_string(),
            encode_map(&BTreeMap::from([
                ("provider".to_string(), payload.provider.clone()),
                (
                    "confidence_millis".to_string(),
                    payload
                        .confidence_millis
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                ),
                ("evidence".to_string(), encode_map(&payload.evidence)),
            ])),
            String::new(),
        ),
        EventPayload::Enforcement(payload) => (
            "enforcement".to_string(),
            encode_map(&BTreeMap::from([
                ("backend".to_string(), payload.backend.clone()),
                ("operation".to_string(), payload.operation.clone()),
                ("decision".to_string(), payload.decision.clone()),
                ("path".to_string(), payload.path.clone().unwrap_or_default()),
                (
                    "rule_id".to_string(),
                    payload.rule_id.clone().unwrap_or_default(),
                ),
                ("result".to_string(), payload.result.clone()),
                ("metadata".to_string(), encode_map(&payload.metadata)),
            ])),
            String::new(),
        ),
    }
}

pub fn decode_event_payload(
    variant: &str,
    fields: &str,
    bytes: &str,
) -> Result<EventPayload, SqlError> {
    let fields = decode_map(fields);
    match variant {
        "process" => Ok(EventPayload::Process(ProcessPayload {
            operation: fields.get("operation").cloned().unwrap_or_default(),
            parent: fields
                .get("parent")
                .filter(|value| !value.is_empty())
                .map(|value| decode_process_identity_inline(value))
                .transpose()?,
            executable: fields
                .get("executable")
                .cloned()
                .filter(|value| !value.is_empty()),
            metadata: fields
                .get("metadata")
                .map(|value| decode_map(value))
                .unwrap_or_default(),
        })),
        "file" => Ok(EventPayload::File(FilePayload {
            operation: fields.get("operation").cloned().unwrap_or_default(),
            path: fields
                .get("path")
                .cloned()
                .filter(|value| !value.is_empty()),
            result: fields
                .get("result")
                .and_then(|value| value.parse::<i32>().ok()),
            metadata: fields
                .get("metadata")
                .map(|value| decode_map(value))
                .unwrap_or_default(),
        })),
        "net" => Ok(EventPayload::Net(NetPayload {
            transport: fields.get("transport").cloned().unwrap_or_default(),
            local: fields
                .get("local")
                .cloned()
                .filter(|value| !value.is_empty()),
            remote: fields
                .get("remote")
                .cloned()
                .filter(|value| !value.is_empty()),
            size: fields
                .get("size")
                .and_then(|value| value.parse::<u64>().ok()),
            result: fields
                .get("result")
                .and_then(|value| value.parse::<i32>().ok()),
            metadata: fields
                .get("metadata")
                .map(|value| decode_map(value))
                .unwrap_or_default(),
        })),
        "ipc" => Ok(EventPayload::Ipc(IpcPayload {
            channel: fields.get("channel").cloned().unwrap_or_default(),
            peer: fields
                .get("peer")
                .cloned()
                .filter(|value| !value.is_empty()),
            size: fields
                .get("size")
                .and_then(|value| value.parse::<u64>().ok()),
            metadata: fields
                .get("metadata")
                .map(|value| decode_map(value))
                .unwrap_or_default(),
        })),
        "stdio" => Ok(EventPayload::Stdio(StdioPayload {
            stream: fields.get("stream").cloned().unwrap_or_default(),
            data: decode_bytes(bytes)?,
            original_size: fields
                .get("original_size")
                .and_then(|value| value.parse::<usize>().ok()),
            truncated: fields
                .get("truncated")
                .map(|value| value == "true")
                .unwrap_or(false),
        })),
        "application" => Ok(EventPayload::Application(ApplicationPayload {
            protocol: fields.get("protocol").cloned().unwrap_or_default(),
            operation: fields.get("operation").cloned().unwrap_or_default(),
            summary: fields.get("summary").cloned().unwrap_or_default(),
            metadata: fields
                .get("metadata")
                .map(|value| decode_map(value))
                .unwrap_or_default(),
        })),
        "resource" => Ok(EventPayload::Resource(ResourcePayload {
            scope: fields.get("scope").cloned().unwrap_or_default(),
            subject: fields.get("subject").cloned().unwrap_or_default(),
            cpu_percent_millis: fields
                .get("cpu_percent_millis")
                .and_then(|value| value.parse::<u64>().ok()),
            rss_kb: fields
                .get("rss_kb")
                .and_then(|value| value.parse::<u64>().ok()),
            virtual_memory_kb: fields
                .get("virtual_memory_kb")
                .and_then(|value| value.parse::<u64>().ok()),
            metadata: fields
                .get("metadata")
                .map(|value| decode_map(value))
                .unwrap_or_default(),
        })),
        "control" => Ok(EventPayload::Control(ControlPayload {
            action: fields.get("action").cloned().unwrap_or_default(),
            detail: fields.get("detail").cloned().unwrap_or_default(),
        })),
        "loss" => Ok(EventPayload::Loss(LossPayload {
            reason: fields.get("reason").cloned().unwrap_or_default(),
            fatal: fields
                .get("fatal")
                .map(|value| value == "true")
                .unwrap_or(false),
        })),
        "label" => Ok(EventPayload::Label(LabelPayload {
            provider: fields.get("provider").cloned().unwrap_or_default(),
            confidence_millis: fields
                .get("confidence_millis")
                .filter(|value| !value.is_empty())
                .and_then(|value| value.parse::<u16>().ok()),
            evidence: fields
                .get("evidence")
                .map(|value| decode_map(value))
                .unwrap_or_default(),
        })),
        "enforcement" => Ok(EventPayload::Enforcement(EnforcementPayload {
            backend: fields.get("backend").cloned().unwrap_or_default(),
            operation: fields.get("operation").cloned().unwrap_or_default(),
            decision: fields.get("decision").cloned().unwrap_or_default(),
            path: fields
                .get("path")
                .cloned()
                .filter(|value| !value.is_empty()),
            rule_id: fields
                .get("rule_id")
                .cloned()
                .filter(|value| !value.is_empty()),
            result: fields.get("result").cloned().unwrap_or_default(),
            metadata: fields
                .get("metadata")
                .map(|value| decode_map(value))
                .unwrap_or_default(),
        })),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_process_identity_inline(identity: &ProcessIdentity) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        identity.pid,
        identity
            .task_id
            .map(|value| value.to_string())
            .unwrap_or_default(),
        identity.start_time_ticks,
        identity
            .pid_namespace
            .as_ref()
            .map(|value| escape(value.as_str()))
            .unwrap_or_default(),
        identity.generation
    )
}

pub(crate) fn decode_process_identity_inline(raw: &str) -> Result<ProcessIdentity, SqlError> {
    let mut parts = raw.split('|');
    let pid = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(SqlError::InvalidQuery)?;
    let task_id = parts
        .next()
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u32>().ok());
    let start_time_ticks = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(SqlError::InvalidQuery)?;
    let pid_namespace = parts
        .next()
        .filter(|value| !value.is_empty())
        .map(|value| NamespaceIdentity::new(unescape(value)));
    let generation = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(SqlError::InvalidQuery)?;
    Ok(ProcessIdentity {
        pid,
        task_id,
        start_time_ticks,
        pid_namespace,
        generation,
        start_unix_seconds: None,
            start_unix_millis: None,
    })
}
