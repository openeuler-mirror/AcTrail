//! Event, process, and diagnostic JSON rendering.

use std::fmt::Write;

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::{DomainEvent, EventPayload, NetPayload};
use model_core::process::{ExitObservationSource, ProcessMembership};

use crate::json;

pub(super) fn event_json(event: &DomainEvent) -> String {
    let mut output = String::from("{");
    json::field(
        &mut output,
        "id",
        &json::number(event.envelope.event_id.get()),
    );
    output.push(',');
    json::field(
        &mut output,
        "display_id",
        &json::string(&event.envelope.event_id.to_string()),
    );
    output.push(',');
    json::field(
        &mut output,
        "domain",
        &json::string(&format!("{:?}", event.envelope.kind)),
    );
    output.push(',');
    json::field(
        &mut output,
        "pid",
        &json::number(event.envelope.process.pid),
    );
    output.push(',');
    json::field(
        &mut output,
        "observed_at",
        &json::time(event.envelope.observed_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "observed_at_unix_nanos",
        &json::time_nanos(event.envelope.observed_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "operation",
        &json::string(&event_operation(event)),
    );
    output.push(',');
    json::field(&mut output, "summary", &json::string(&event_summary(event)));
    output.push(',');
    json::field(&mut output, "metadata", &json::map(&event_metadata(event)));
    output.push('}');
    output
}

pub(super) fn process_json(membership: &ProcessMembership) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "pid", &json::number(membership.identity.pid));
    output.push(',');
    json::field(
        &mut output,
        "identity",
        &json::process(&membership.identity),
    );
    output.push(',');
    json::field(
        &mut output,
        "parent_pid",
        &json::optional_number(membership.inherited_from.as_ref().map(|parent| parent.pid)),
    );
    output.push(',');
    json::field(
        &mut output,
        "observed_at",
        &json::optional_time(membership.observed_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "observed_at_unix_nanos",
        &json::optional_time_nanos(membership.observed_at),
    );
    output.push(',');
    json::field(
        &mut output,
        "state",
        &json::string(&format!("{:?}", membership.state)),
    );
    output.push(',');
    json::field(
        &mut output,
        "exit_code",
        &json::optional_number(
            membership
                .exit_status
                .as_ref()
                .and_then(|status| status.code),
        ),
    );
    output.push(',');
    json::field(
        &mut output,
        "exit_observed_at",
        &json::optional_time(
            membership
                .exit_status
                .as_ref()
                .map(|status| status.observed_at),
        ),
    );
    output.push(',');
    json::field(
        &mut output,
        "exit_observed_at_unix_nanos",
        &json::optional_time_nanos(
            membership
                .exit_status
                .as_ref()
                .map(|status| status.observed_at),
        ),
    );
    output.push(',');
    json::field(
        &mut output,
        "exit_observation_source",
        &json::optional_string(
            membership
                .exit_status
                .as_ref()
                .and_then(|status| status.source)
                .map(exit_observation_source_label),
        ),
    );
    output.push('}');
    output
}

fn exit_observation_source_label(source: ExitObservationSource) -> &'static str {
    match source {
        ExitObservationSource::Event => "event",
        ExitObservationSource::Reconciled => "reconciled",
    }
}

pub(super) fn diagnostic_json(diagnostic: &DiagnosticRecord) -> String {
    let mut output = String::from("{");
    json::field(
        &mut output,
        "id",
        &json::number(diagnostic.diagnostic_id.get()),
    );
    output.push(',');
    json::field(
        &mut output,
        "severity",
        &json::string(&format!("{:?}", diagnostic.severity)),
    );
    output.push(',');
    json::field(
        &mut output,
        "kind",
        &json::string(&format!("{:?}", diagnostic.kind)),
    );
    output.push(',');
    json::field(&mut output, "message", &json::string(&diagnostic.message));
    output.push(',');
    json::field(&mut output, "metadata", &json::map(&diagnostic.metadata));
    output.push('}');
    output
}

pub(super) fn event_counts_from_variants(
    variants: &std::collections::BTreeMap<String, usize>,
) -> EventCounts {
    let mut counts = EventCounts::default();
    for (variant, count) in variants {
        counts.events += count;
        match variant.as_str() {
            "process" => counts.process += count,
            "net" => counts.net += count,
            "file" => counts.file += count,
            "ipc" => counts.ipc += count,
            "stdio" => counts.stdio += count,
            "application" => counts.application += count,
            "resource" => counts.resource += count,
            "control" => counts.control += count,
            "loss" => counts.loss += count,
            "label" => counts.label += count,
            "enforcement" => counts.enforcement += count,
            _ => {}
        }
    }
    counts
}

pub(super) fn event_counts(events: &[DomainEvent]) -> EventCounts {
    let mut counts = EventCounts::default();
    for event in events {
        counts.events += 1;
        match event.payload {
            EventPayload::Process(_) => counts.process += 1,
            EventPayload::Net(_) => counts.net += 1,
            EventPayload::File(_) => counts.file += 1,
            EventPayload::Ipc(_) => counts.ipc += 1,
            EventPayload::Stdio(_) => counts.stdio += 1,
            EventPayload::Application(_) => counts.application += 1,
            EventPayload::Resource(_) => counts.resource += 1,
            EventPayload::Control(_) => counts.control += 1,
            EventPayload::Loss(_) => counts.loss += 1,
            EventPayload::Label(_) => counts.label += 1,
            EventPayload::Enforcement(_) => counts.enforcement += 1,
        }
    }
    counts
}

pub(super) fn counts_json(
    counts: &EventCounts,
    retained_payload_bytes: u64,
    payload_segments: usize,
) -> String {
    format!(
        "{{\"events\":{},\"process\":{},\"net\":{},\"file\":{},\"ipc\":{},\"stdio\":{},\"application\":{},\"resource\":{},\"control\":{},\"loss\":{},\"label\":{},\"enforcement\":{},\"payloads\":{},\"retained_payload_bytes\":{}}}",
        counts.events,
        counts.process,
        counts.net,
        counts.file,
        counts.ipc,
        counts.stdio,
        counts.application,
        counts.resource,
        counts.control,
        counts.loss,
        counts.label,
        counts.enforcement,
        payload_segments,
        retained_payload_bytes
    )
}

fn event_operation(event: &DomainEvent) -> String {
    match &event.payload {
        EventPayload::Process(payload) => payload.operation.clone(),
        EventPayload::Net(payload) => payload
            .metadata
            .get("operation")
            .cloned()
            .unwrap_or_default(),
        EventPayload::File(payload) => payload.operation.clone(),
        EventPayload::Ipc(payload) => payload
            .metadata
            .get("operation")
            .cloned()
            .unwrap_or_default(),
        EventPayload::Stdio(payload) => payload.stream.clone(),
        EventPayload::Application(payload) => payload.operation.clone(),
        EventPayload::Resource(payload) => payload.scope.clone(),
        EventPayload::Control(payload) => payload.action.clone(),
        EventPayload::Loss(payload) => payload.reason.clone(),
        EventPayload::Label(payload) => payload.provider.clone(),
        EventPayload::Enforcement(payload) => payload.operation.clone(),
    }
}

fn event_summary(event: &DomainEvent) -> String {
    match &event.payload {
        EventPayload::Process(payload) => payload
            .executable
            .clone()
            .or_else(|| payload.metadata.get("target_pid").cloned())
            .unwrap_or_default(),
        EventPayload::Net(payload) => network_summary(payload),
        EventPayload::File(payload) => payload.path.clone().unwrap_or_default(),
        EventPayload::Ipc(payload) => payload.peer.clone().unwrap_or_default(),
        EventPayload::Application(payload) => payload.summary.clone(),
        EventPayload::Resource(payload) => resource_summary(payload),
        EventPayload::Label(payload) => payload
            .confidence_millis
            .map(|value| format!("confidence_millis={value}"))
            .unwrap_or_default(),
        EventPayload::Stdio(payload) => format!("{} bytes", payload.data.len()),
        EventPayload::Control(payload) => payload.detail.clone(),
        EventPayload::Loss(payload) => format!("fatal={}", payload.fatal),
        EventPayload::Enforcement(payload) => format!(
            "{} {} {}",
            payload.decision,
            payload.path.clone().unwrap_or_default(),
            payload.result
        ),
    }
}

fn event_metadata(event: &DomainEvent) -> std::collections::BTreeMap<String, String> {
    match &event.payload {
        EventPayload::Process(payload) => payload.metadata.clone(),
        EventPayload::Net(payload) => payload.metadata.clone(),
        EventPayload::File(payload) => payload.metadata.clone(),
        EventPayload::Ipc(payload) => payload.metadata.clone(),
        EventPayload::Application(payload) => payload.metadata.clone(),
        EventPayload::Resource(payload) => resource_metadata(payload),
        EventPayload::Label(payload) => payload.evidence.clone(),
        EventPayload::Enforcement(payload) => enforcement_metadata(payload),
        _ => std::collections::BTreeMap::new(),
    }
}

fn enforcement_metadata(
    payload: &model_core::event::EnforcementPayload,
) -> std::collections::BTreeMap<String, String> {
    let mut metadata = payload.metadata.clone();
    metadata.insert("backend".to_string(), payload.backend.clone());
    metadata.insert("decision".to_string(), payload.decision.clone());
    metadata.insert("operation".to_string(), payload.operation.clone());
    metadata.insert("result".to_string(), payload.result.clone());
    if let Some(path) = &payload.path {
        metadata.insert("path".to_string(), path.clone());
    }
    if let Some(rule_id) = &payload.rule_id {
        metadata.insert("rule_id".to_string(), rule_id.clone());
    }
    metadata
}

fn resource_metadata(
    payload: &model_core::event::ResourcePayload,
) -> std::collections::BTreeMap<String, String> {
    let mut metadata = payload.metadata.clone();
    metadata.insert("subject".to_string(), payload.subject.clone());
    if let Some(cpu_percent_millis) = payload.cpu_percent_millis {
        metadata.insert(
            "cpu_percent".to_string(),
            format_percent_millis(cpu_percent_millis),
        );
        metadata.insert(
            "cpu_percent_millis".to_string(),
            cpu_percent_millis.to_string(),
        );
    }
    if let Some(rss_kb) = payload.rss_kb {
        metadata.insert("rss_kb".to_string(), rss_kb.to_string());
    }
    if let Some(virtual_memory_kb) = payload.virtual_memory_kb {
        metadata.insert(
            "virtual_memory_kb".to_string(),
            virtual_memory_kb.to_string(),
        );
    }
    metadata
}

fn resource_summary(payload: &model_core::event::ResourcePayload) -> String {
    format!(
        "{} cpu={} rss_kb={}",
        payload.subject,
        payload
            .cpu_percent_millis
            .map(format_percent_millis)
            .unwrap_or_default(),
        payload
            .rss_kb
            .map(|value| value.to_string())
            .unwrap_or_default()
    )
}

fn network_summary(payload: &NetPayload) -> String {
    let mut summary = String::new();
    let _ = write!(
        summary,
        "{} -> {}",
        payload.local.clone().unwrap_or_default(),
        payload.remote.clone().unwrap_or_default()
    );
    summary
}

fn format_percent_millis(value: u64) -> String {
    format!("{}.{:03}", value / 1000, value % 1000)
}

#[derive(Default)]
pub(super) struct EventCounts {
    events: usize,
    process: usize,
    net: usize,
    file: usize,
    ipc: usize,
    stdio: usize,
    application: usize,
    resource: usize,
    control: usize,
    loss: usize,
    label: usize,
    enforcement: usize,
}
