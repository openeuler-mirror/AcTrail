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
