use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::event::{DomainEvent, EventPayload, LabelPayload, NetPayload};
use model_core::process::ProcessIdentity;

use crate::command::RowLimit;
use crate::table::Table;

pub(super) fn render_network(events: Vec<DomainEvent>, row_limit: Option<RowLimit>) -> String {
    let mut table = Table::new(&[
        "EVENT",
        "PID",
        "PROVIDER",
        "SIDE",
        "OPERATION",
        "LOCAL",
        "REMOTE",
        "RESULT",
    ]);
    for row in super::limit_vec(network_rows(events), row_limit) {
        table.push(vec![
            row.event_id,
            row.pid,
            row.provider,
            row.side,
            row.operation,
            row.local,
            row.remote,
            row.result,
        ]);
    }
    super::render_table(table, "no network events")
}

pub(super) fn network_detail(payload: &NetPayload) -> String {
    let operation = network_operation(payload);
    format!(
        "side={} local={} remote={} result={}",
        network_side(&operation, payload),
        payload.local.clone().unwrap_or_default(),
        payload.remote.clone().unwrap_or_default(),
        network_result(&operation, payload)
    )
}

struct NetworkRow {
    event_id: String,
    pid: String,
    provider: String,
    side: String,
    operation: String,
    local: String,
    remote: String,
    result: String,
}

struct PendingLabelTarget {
    row_index: usize,
    process: ProcessIdentity,
    observed_at: SystemTime,
}

fn network_rows(events: Vec<DomainEvent>) -> Vec<NetworkRow> {
    let mut rows = Vec::new();
    let mut pending_label_target = None;

    for event in events {
        let event_process = event.envelope.process;
        let observed_at = event.envelope.observed_at;
        match event.payload {
            EventPayload::Net(payload) => {
                let operation = network_operation(&payload);
                let local = payload.local.clone().unwrap_or_default();
                let remote = payload.remote.clone().unwrap_or_default();
                let result = network_result(&operation, &payload);
                rows.push(NetworkRow {
                    event_id: event.envelope.event_id.to_string(),
                    pid: event.envelope.process.get().to_string(),
                    provider: String::new(),
                    side: network_side(&operation, &payload),
                    operation,
                    local,
                    remote,
                    result,
                });
                pending_label_target = Some(PendingLabelTarget {
                    row_index: rows.len() - 1,
                    process: event_process,
                    observed_at,
                });
            }
            EventPayload::Label(label) => {
                if let Some(target) = pending_label_target.take() {
                    if label_belongs_to_row(
                        &label,
                        event_process,
                        observed_at,
                        &rows[target.row_index],
                        &target,
                    ) {
                        rows[target.row_index].provider = label.provider;
                    }
                }
            }
            _ => {}
        }
    }

    rows
}

fn label_belongs_to_row(
    label: &LabelPayload,
    event_process: ProcessIdentity,
    observed_at: SystemTime,
    row: &NetworkRow,
    target: &PendingLabelTarget,
) -> bool {
    event_process == target.process
        && observed_at == target.observed_at
        && evidence_matches("operation", &row.operation, &label.evidence)
        && evidence_matches("local", &row.local, &label.evidence)
        && evidence_matches_optional("remote", &row.remote, &label.evidence)
}

fn evidence_matches(key: &str, expected: &str, evidence: &BTreeMap<String, String>) -> bool {
    evidence.get(key).is_some_and(|actual| actual == expected)
}

fn evidence_matches_optional(
    key: &str,
    expected: &str,
    evidence: &BTreeMap<String, String>,
) -> bool {
    expected.is_empty() || evidence_matches(key, expected, evidence)
}

fn network_operation(payload: &NetPayload) -> String {
    payload
        .metadata
        .get("operation")
        .cloned()
        .unwrap_or_default()
}

fn network_side(operation: &str, payload: &NetPayload) -> String {
    match operation {
        "connect" => "client-open".to_string(),
        "accept" => "server-accept".to_string(),
        _ => payload
            .metadata
            .get("direction")
            .cloned()
            .unwrap_or_default(),
    }
}

fn network_result(operation: &str, payload: &NetPayload) -> String {
    let Some(result) = payload.result else {
        return String::new();
    };
    if result < 0 {
        let errno = result.checked_abs().unwrap_or(i32::MAX);
        let message = std::io::Error::from_raw_os_error(errno).to_string();
        return format!("errno={errno} {message}");
    }
    match operation {
        "accept" => format!("fd={result}"),
        "send" | "recv" => payload
            .size
            .map(|size| format!("bytes={size}"))
            .unwrap_or_else(|| format!("bytes={result}")),
        "connect" | "bind" | "listen" => "ok".to_string(),
        _ => format!("return={result}"),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    use model_core::event::{
        DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, LabelPayload, NetPayload,
    };
    use model_core::ids::{CollectorName, EventId, TraceId};
    use model_core::process::ProcessIdentity;

    use super::render_network;

    const TRACE_ID_RAW: u64 = 1;
    const NET_EVENT_ID_RAW: u64 = 10;
    const LABEL_EVENT_ID_RAW: u64 = 11;
    const PID: u32 = 4242;
    const CONFIDENCE_MILLIS: u16 = 950;

    #[test]
    fn network_render_includes_adjacent_provider_label() {
        let observed_at = SystemTime::UNIX_EPOCH;
        let output = render_network(
            vec![
                net_event(EventId::new(NET_EVENT_ID_RAW), observed_at),
                label_event(EventId::new(LABEL_EVENT_ID_RAW), observed_at),
            ],
            None,
        );

        assert!(output.contains("PROVIDER"));
        assert!(output.contains("actrail-local-tcp"));
        assert!(output.contains("client-open"));
    }

    fn net_event(event_id: EventId, observed_at: SystemTime) -> DomainEvent {
        let mut metadata = BTreeMap::new();
        metadata.insert("operation".to_string(), "connect".to_string());
        metadata.insert("direction".to_string(), "outbound".to_string());
        DomainEvent::new(
            envelope(event_id, observed_at, EventKind::Net),
            EventPayload::Net(NetPayload {
                transport: "tcp".to_string(),
                local: Some("127.0.0.1:40000".to_string()),
                remote: Some("127.0.0.1:50000".to_string()),
                size: None,
                result: Some(0),
                metadata,
            }),
        )
    }

    fn label_event(event_id: EventId, observed_at: SystemTime) -> DomainEvent {
        DomainEvent::new(
            envelope(event_id, observed_at, EventKind::Label),
            EventPayload::Label(LabelPayload {
                provider: "actrail-local-tcp".to_string(),
                confidence_millis: Some(CONFIDENCE_MILLIS),
                evidence: label_evidence(),
            }),
        )
    }

    fn label_evidence() -> BTreeMap<String, String> {
        let mut evidence = BTreeMap::new();
        evidence.insert("operation".to_string(), "connect".to_string());
        evidence.insert("local".to_string(), "127.0.0.1:40000".to_string());
        evidence.insert("remote".to_string(), "127.0.0.1:50000".to_string());
        evidence.insert("transport".to_string(), "tcp".to_string());
        evidence
    }

    fn envelope(event_id: EventId, observed_at: SystemTime, kind: EventKind) -> EventEnvelope {
        EventEnvelope {
            event_id,
            trace_id: TraceId::new(TRACE_ID_RAW),
            observed_at,
            process: ProcessIdentity::new(PID as u64),
            collector: CollectorName::new("ebpf"),
            kind,
            flags: EventFlags::clean(),
        }
    }
}
