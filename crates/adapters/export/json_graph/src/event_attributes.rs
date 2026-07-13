//! Event title and attribute projection for graph exports.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use model_core::event::{
    ApplicationPayload, DomainEvent, EnforcementPayload, EventPayload, FilePayload, IpcPayload,
    LabelPayload, LossPayload, NetPayload, ProcessPayload, ResourcePayload, StdioPayload,
};
use model_core::process::ProcessIdentity;

pub fn event_attributes(event: &DomainEvent) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "event_id".to_string(),
        event.envelope.event_id.get().to_string(),
    );
    attributes.insert("kind".to_string(), format!("{:?}", event.envelope.kind));
    attributes.insert(
        "collector".to_string(),
        event.envelope.collector.to_string(),
    );
    insert_time(&mut attributes, "observed_at", event.envelope.observed_at);
    insert_process_identity(&mut attributes, "process_", &event.envelope.process);
    attributes.insert(
        "bootstrap_observed".to_string(),
        event.envelope.flags.bootstrap_observed.to_string(),
    );
    attributes.insert(
        "metadata_partial".to_string(),
        event.envelope.flags.metadata_partial.to_string(),
    );
    attributes.insert(
        "policy_modified".to_string(),
        event.envelope.flags.policy_modified.to_string(),
    );
    attributes.insert(
        "policy_verdict".to_string(),
        format!("{:?}", event.policy.verdict),
    );
    if let Some(note) = &event.policy.note {
        attributes.insert("policy_note".to_string(), note.clone());
    }
    attributes.insert(
        "policy_redaction_count".to_string(),
        event.policy.redactions.len().to_string(),
    );
    attributes.insert(
        "policy_truncation_count".to_string(),
        event.policy.truncations.len().to_string(),
    );
    insert_payload_attributes(&mut attributes, &event.payload);
    attributes
}

pub fn event_title(event: &DomainEvent) -> String {
    match &event.payload {
        EventPayload::Process(payload) => format!("Process {}", payload.operation),
        EventPayload::File(payload) => payload
            .path
            .as_ref()
            .map(|path| format!("File {} {}", payload.operation, path))
            .unwrap_or_else(|| format!("File {}", payload.operation)),
        EventPayload::Net(payload) => {
            let operation = payload
                .metadata
                .get("operation")
                .map(String::as_str)
                .unwrap_or("net");
            payload
                .remote
                .as_ref()
                .or(payload.local.as_ref())
                .map(|endpoint| format!("Net {operation} {endpoint}"))
                .unwrap_or_else(|| format!("Net {operation}"))
        }
        EventPayload::Ipc(payload) => format!("IPC {}", payload.channel),
        EventPayload::Stdio(payload) => format!("Stdio {}", payload.stream),
        EventPayload::Application(payload) => {
            format!("Application {} {}", payload.protocol, payload.operation)
        }
        EventPayload::Resource(payload) => format!("Resource {}", payload.scope),
        EventPayload::Control(payload) => format!("Control {}", payload.action),
        EventPayload::Loss(payload) => format!("Loss {}", payload.reason),
        EventPayload::Label(payload) => format!("Label {}", payload.provider),
        EventPayload::Enforcement(payload) => {
            format!("Enforcement {} {}", payload.decision, payload.operation)
        }
    }
}

pub fn insert_process_identity(
    attributes: &mut BTreeMap<String, String>,
    prefix: &str,
    identity: &ProcessIdentity,
) {
    attributes.insert(format!("{prefix}process_id"), identity.get().to_string());
}

pub fn insert_time(attributes: &mut BTreeMap<String, String>, prefix: &str, time: SystemTime) {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            attributes.insert(
                format!("{prefix}_unix_seconds"),
                duration.as_secs().to_string(),
            );
            attributes.insert(
                format!("{prefix}_unix_nanos"),
                duration.subsec_nanos().to_string(),
            );
        }
        Err(error) => {
            attributes.insert(format!("{prefix}_before_unix_epoch"), error.to_string());
        }
    }
}

fn insert_payload_attributes(attributes: &mut BTreeMap<String, String>, payload: &EventPayload) {
    match payload {
        EventPayload::Process(payload) => insert_process_payload(attributes, payload),
        EventPayload::File(payload) => insert_file_payload(attributes, payload),
        EventPayload::Net(payload) => insert_net_payload(attributes, payload),
        EventPayload::Ipc(payload) => insert_ipc_payload(attributes, payload),
        EventPayload::Stdio(payload) => insert_stdio_payload(attributes, payload),
        EventPayload::Application(payload) => insert_application_payload(attributes, payload),
        EventPayload::Resource(payload) => insert_resource_payload(attributes, payload),
        EventPayload::Control(payload) => {
            attributes.insert("action".to_string(), payload.action.clone());
            attributes.insert("detail".to_string(), payload.detail.clone());
        }
        EventPayload::Loss(payload) => insert_loss_payload(attributes, payload),
        EventPayload::Label(payload) => insert_label_payload(attributes, payload),
        EventPayload::Enforcement(payload) => insert_enforcement_payload(attributes, payload),
    }
}

fn insert_process_payload(attributes: &mut BTreeMap<String, String>, payload: &ProcessPayload) {
    attributes.insert("operation".to_string(), payload.operation.clone());
    if let Some(parent) = &payload.parent {
        insert_process_identity(attributes, "parent_", parent);
    }
    if let Some(executable) = &payload.executable {
        attributes.insert("executable".to_string(), executable.clone());
    }
    insert_metadata(attributes, "metadata.", &payload.metadata);
}

fn insert_file_payload(attributes: &mut BTreeMap<String, String>, payload: &FilePayload) {
    attributes.insert("operation".to_string(), payload.operation.clone());
    if let Some(path) = &payload.path {
        attributes.insert("path".to_string(), path.clone());
    }
    if let Some(result) = payload.result {
        attributes.insert("result".to_string(), result.to_string());
    }
    insert_metadata(attributes, "metadata.", &payload.metadata);
}

fn insert_net_payload(attributes: &mut BTreeMap<String, String>, payload: &NetPayload) {
    attributes.insert("transport".to_string(), payload.transport.clone());
    insert_if_present(attributes, "local", payload.local.as_ref());
    insert_if_present(attributes, "remote", payload.remote.as_ref());
    if let Some(size) = payload.size {
        attributes.insert("size".to_string(), size.to_string());
    }
    if let Some(result) = payload.result {
        attributes.insert("result".to_string(), result.to_string());
    }
    insert_metadata(attributes, "metadata.", &payload.metadata);
    if let Some(operation) = payload.metadata.get("operation") {
        attributes.insert("operation".to_string(), operation.clone());
    }
    if let Some(direction) = payload.metadata.get("direction") {
        attributes.insert("direction".to_string(), direction.clone());
    }
    if let Some(fd) = payload.metadata.get("fd") {
        attributes.insert("fd".to_string(), fd.clone());
    }
}

fn insert_ipc_payload(attributes: &mut BTreeMap<String, String>, payload: &IpcPayload) {
    attributes.insert("channel".to_string(), payload.channel.clone());
    insert_if_present(attributes, "peer", payload.peer.as_ref());
    if let Some(size) = payload.size {
        attributes.insert("size".to_string(), size.to_string());
    }
    insert_metadata(attributes, "metadata.", &payload.metadata);
}

fn insert_stdio_payload(attributes: &mut BTreeMap<String, String>, payload: &StdioPayload) {
    attributes.insert("stream".to_string(), payload.stream.clone());
    attributes.insert(
        "data_byte_count".to_string(),
        payload.data.len().to_string(),
    );
    if let Some(original_size) = payload.original_size {
        attributes.insert("original_size".to_string(), original_size.to_string());
    }
    attributes.insert("truncated".to_string(), payload.truncated.to_string());
}

fn insert_application_payload(
    attributes: &mut BTreeMap<String, String>,
    payload: &ApplicationPayload,
) {
    attributes.insert("protocol".to_string(), payload.protocol.clone());
    attributes.insert("operation".to_string(), payload.operation.clone());
    attributes.insert("summary".to_string(), payload.summary.clone());
    insert_metadata(attributes, "metadata.", &payload.metadata);
}

fn insert_resource_payload(attributes: &mut BTreeMap<String, String>, payload: &ResourcePayload) {
    attributes.insert("scope".to_string(), payload.scope.clone());
    attributes.insert("subject".to_string(), payload.subject.clone());
    if let Some(cpu_percent_millis) = payload.cpu_percent_millis {
        attributes.insert(
            "cpu_percent_millis".to_string(),
            cpu_percent_millis.to_string(),
        );
    }
    if let Some(rss_kb) = payload.rss_kb {
        attributes.insert("rss_kb".to_string(), rss_kb.to_string());
    }
    if let Some(virtual_memory_kb) = payload.virtual_memory_kb {
        attributes.insert(
            "virtual_memory_kb".to_string(),
            virtual_memory_kb.to_string(),
        );
    }
    insert_metadata(attributes, "metadata.", &payload.metadata);
}

fn insert_loss_payload(attributes: &mut BTreeMap<String, String>, payload: &LossPayload) {
    attributes.insert("reason".to_string(), payload.reason.clone());
    attributes.insert("fatal".to_string(), payload.fatal.to_string());
}

fn insert_label_payload(attributes: &mut BTreeMap<String, String>, payload: &LabelPayload) {
    attributes.insert("provider".to_string(), payload.provider.clone());
    if let Some(confidence_millis) = payload.confidence_millis {
        attributes.insert(
            "confidence_millis".to_string(),
            confidence_millis.to_string(),
        );
    }
    insert_metadata(attributes, "evidence.", &payload.evidence);
}

fn insert_enforcement_payload(
    attributes: &mut BTreeMap<String, String>,
    payload: &EnforcementPayload,
) {
    attributes.insert("backend".to_string(), payload.backend.clone());
    attributes.insert("operation".to_string(), payload.operation.clone());
    attributes.insert("decision".to_string(), payload.decision.clone());
    attributes.insert("result".to_string(), payload.result.clone());
    insert_if_present(attributes, "path", payload.path.as_ref());
    insert_if_present(attributes, "rule_id", payload.rule_id.as_ref());
    insert_metadata(attributes, "metadata.", &payload.metadata);
}

fn insert_metadata(
    attributes: &mut BTreeMap<String, String>,
    prefix: &str,
    metadata: &BTreeMap<String, String>,
) {
    for (key, value) in metadata {
        attributes.insert(format!("{prefix}{key}"), value.clone());
    }
}

fn insert_if_present(
    attributes: &mut BTreeMap<String, String>,
    key: &'static str,
    value: Option<&String>,
) {
    if let Some(value) = value {
        attributes.insert(key.to_string(), value.clone());
    }
}
