//! Text rendering for storage-backed viewer output.

#[path = "render/network.rs"]
mod network;
#[path = "render/payload.rs"]
mod payload;

use std::time::{SystemTime, UNIX_EPOCH};

use model_core::diagnostics::DiagnosticRecord;
use model_core::event::{DomainEvent, EventPayload, NetPayload};
use model_core::payload::PayloadSegment;
use model_core::process::ProcessMembership;
use model_core::trace::TraceRecord;
use semantic_action::SemanticAction;
use store_snapshot_contract::view::SnapshotView;

use crate::command::{PayloadFormat, RowLimit};
use crate::table::Table;

pub(super) fn render_summary(snapshot: &SnapshotView) -> String {
    let net_count = snapshot
        .events
        .iter()
        .filter(|event| is_network_event(event))
        .count();
    format!(
        "Trace {} title={} state={:?} health={:?} profile={}\nroot_pid={} processes={} events={} network_events={} diagnostics={}",
        snapshot.trace.trace_id,
        snapshot.trace.display_name,
        snapshot.trace.lifecycle_state,
        snapshot.trace.health,
        snapshot.trace.profile_name,
        snapshot.trace.root_process_identity.pid,
        snapshot.memberships.len(),
        snapshot.events.len(),
        net_count,
        snapshot.diagnostics.len()
    )
}

pub(super) fn render_traces(traces: Vec<TraceRecord>, row_limit: Option<RowLimit>) -> String {
    let mut table = Table::new(&["TRACE", "NAME", "ROOT_PID", "STATE", "HEALTH", "CREATED"]);
    for trace in limit_vec(traces, row_limit) {
        table.push(vec![
            trace.trace_id.to_string(),
            trace.display_name.to_string(),
            trace.root_process_identity.pid.to_string(),
            format!("{:?}", trace.lifecycle_state),
            format!("{:?}", trace.health),
            format_time(trace.timings.created_at),
        ]);
    }
    render_table(table, "no traces")
}

pub(super) fn render_processes(
    memberships: Vec<ProcessMembership>,
    row_limit: Option<RowLimit>,
) -> String {
    let mut table = Table::new(&["PID", "STATE", "EXIT", "GENERATION", "PARENT_PID"]);
    for membership in limit_vec(memberships, row_limit) {
        table.push(vec![
            membership.identity.pid.to_string(),
            format!("{:?}", membership.state),
            membership
                .exit_status
                .and_then(|status| status.code)
                .map(|code| code.to_string())
                .unwrap_or_default(),
            membership.identity.generation.to_string(),
            membership
                .inherited_from
                .map(|parent| parent.pid.to_string())
                .unwrap_or_default(),
        ]);
    }
    render_table(table, "no processes")
}

pub(super) fn render_events(events: Vec<DomainEvent>, row_limit: Option<RowLimit>) -> String {
    let mut table = Table::new(&["EVENT", "DOMAIN", "PID", "OPERATION", "DETAIL"]);
    for event in limit_vec(events, row_limit) {
        table.push(vec![
            event.envelope.event_id.to_string(),
            format!("{:?}", event.envelope.kind),
            event.envelope.process.pid.to_string(),
            event_operation(&event),
            event_detail(&event),
        ]);
    }
    render_table(table, "no events")
}

pub(super) fn render_network(events: Vec<DomainEvent>, row_limit: Option<RowLimit>) -> String {
    network::render_network(events, row_limit)
}

pub(super) fn render_payloads(segments: Vec<PayloadSegment>) -> String {
    payload::render_payloads(segments)
}

pub(super) fn render_payload(segment: PayloadSegment, format: PayloadFormat) -> String {
    payload::render_payload(segment, format)
}

pub(super) fn render_semantic_actions(
    actions: Vec<SemanticAction>,
    row_limit: Option<RowLimit>,
) -> String {
    let mut table = Table::new(&[
        "ACTION",
        "KIND",
        "PID",
        "STATUS",
        "COMPLETENESS",
        "EVIDENCE",
        "TITLE",
    ]);
    for action in limit_vec(actions, row_limit) {
        table.push(vec![
            action.action_id,
            action.kind.as_str().to_string(),
            action.process.pid.to_string(),
            action.status.as_str().to_string(),
            action.completeness.as_str().to_string(),
            action.evidence.len().to_string(),
            action.title,
        ]);
    }
    render_table(table, "no semantic actions")
}

pub(super) fn render_diagnostics(
    diagnostics: Vec<DiagnosticRecord>,
    row_limit: Option<RowLimit>,
) -> String {
    let mut table = Table::new(&["DIAG", "SEVERITY", "KIND", "MESSAGE"]);
    for diagnostic in limit_vec(diagnostics, row_limit) {
        table.push(vec![
            diagnostic.diagnostic_id.to_string(),
            format!("{:?}", diagnostic.severity),
            format!("{:?}", diagnostic.kind),
            diagnostic.message,
        ]);
    }
    render_table(table, "no diagnostics")
}

pub(super) fn reject_limit(
    row_limit: Option<RowLimit>,
    command: &'static str,
) -> Result<(), String> {
    if row_limit.is_some() {
        return Err(format!("{command} does not accept --head or --tail"));
    }
    Ok(())
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
        EventPayload::Ipc(payload) => payload.channel.clone(),
        EventPayload::Stdio(payload) => payload.stream.clone(),
        EventPayload::Application(payload) => payload.operation.clone(),
        EventPayload::Resource(payload) => payload.scope.clone(),
        EventPayload::Control(payload) => payload.action.clone(),
        EventPayload::Loss(payload) => payload.reason.clone(),
        EventPayload::Label(payload) => payload.provider.clone(),
        EventPayload::Enforcement(payload) => payload.operation.clone(),
    }
}

fn event_detail(event: &DomainEvent) -> String {
    match &event.payload {
        EventPayload::Process(payload) => process_detail(payload),
        EventPayload::Net(payload) => network_detail(payload),
        EventPayload::File(payload) => file_detail(payload),
        EventPayload::Ipc(payload) => ipc_detail(payload),
        EventPayload::Application(payload) => application_detail(payload),
        EventPayload::Resource(payload) => resource_detail(payload),
        EventPayload::Label(payload) => label_detail(payload),
        EventPayload::Enforcement(payload) => enforcement_detail(payload),
        _ => String::new(),
    }
}

fn process_detail(payload: &model_core::event::ProcessPayload) -> String {
    match payload.operation.as_str() {
        "fork" => payload
            .parent
            .as_ref()
            .map(|parent| format!("parent={}", parent.pid))
            .unwrap_or_default(),
        "exec" => payload
            .executable
            .clone()
            .or_else(|| payload.metadata.get("executable").cloned())
            .unwrap_or_else(|| "unavailable".to_string()),
        "exit" => payload
            .metadata
            .get("exit_code")
            .map(|code| format!("code={code}"))
            .unwrap_or_default(),
        "signal" => format!(
            "signal={} target_pid={} result={}",
            payload.metadata.get("signal").cloned().unwrap_or_default(),
            payload
                .metadata
                .get("target_pid")
                .cloned()
                .unwrap_or_default(),
            payload.metadata.get("result").cloned().unwrap_or_default()
        ),
        _ => String::new(),
    }
}

fn is_network_event(event: &DomainEvent) -> bool {
    matches!(event.payload, EventPayload::Net(_))
}

fn network_detail(payload: &NetPayload) -> String {
    network::network_detail(payload)
}

fn file_detail(payload: &model_core::event::FilePayload) -> String {
    let result = payload
        .result
        .map(|value| value.to_string())
        .unwrap_or_default();
    format!(
        "path={} result={}",
        payload.path.clone().unwrap_or_default(),
        result
    )
}

fn ipc_detail(payload: &model_core::event::IpcPayload) -> String {
    let operation = payload
        .metadata
        .get("operation")
        .cloned()
        .unwrap_or_default();
    let size = payload
        .size
        .map(|value| value.to_string())
        .unwrap_or_default();
    format!(
        "channel={} operation={} peer={} size={}",
        payload.channel,
        operation,
        payload.peer.clone().unwrap_or_default(),
        size
    )
}

fn application_detail(payload: &model_core::event::ApplicationPayload) -> String {
    format!("{} {}", payload.protocol, payload.summary)
}

fn resource_detail(payload: &model_core::event::ResourcePayload) -> String {
    format!(
        "subject={} cpu_percent={} rss_kb={} virtual_memory_kb={}",
        payload.subject,
        payload
            .cpu_percent_millis
            .map(format_percent_millis)
            .unwrap_or_default(),
        payload
            .rss_kb
            .map(|value| value.to_string())
            .unwrap_or_default(),
        payload
            .virtual_memory_kb
            .map(|value| value.to_string())
            .unwrap_or_default()
    )
}

fn label_detail(payload: &model_core::event::LabelPayload) -> String {
    let confidence = payload
        .confidence_millis
        .map(|value| value.to_string())
        .unwrap_or_default();
    let evidence = payload
        .evidence
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("confidence_millis={confidence} evidence={evidence}")
}

fn enforcement_detail(payload: &model_core::event::EnforcementPayload) -> String {
    format!(
        "decision={} path={} rule_id={} result={} backend={}",
        payload.decision,
        payload.path.clone().unwrap_or_default(),
        payload.rule_id.clone().unwrap_or_default(),
        payload.result,
        payload.backend
    )
}

fn format_percent_millis(value: u64) -> String {
    format!("{}.{:03}", value / 1000, value % 1000)
}

fn limit_vec<T>(mut items: Vec<T>, row_limit: Option<RowLimit>) -> Vec<T> {
    match row_limit {
        Some(RowLimit::Head(count)) => {
            items.truncate(count);
            items
        }
        Some(RowLimit::Tail(count)) if items.len() > count => items.split_off(items.len() - count),
        Some(RowLimit::Tail(_)) | None => items,
    }
}

fn render_table(table: Table, empty_message: &str) -> String {
    if table.is_empty() {
        empty_message.to_string()
    } else {
        table.render()
    }
}

fn format_time(time: SystemTime) -> String {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "before-unix-epoch".to_string(),
    }
}
