//! Ingest-time diagnostic shaping and escalation boundaries.

use std::time::{SystemTime, UNIX_EPOCH};

use collector_event::{RawCollectorEvent, RawObservationPayload};
use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::ids::{DiagnosticId, TraceId};
use model_core::policy::PolicyVerdict;
use model_core::process::ProcessIdentity;
use policy_evaluate_contract::decision::PolicyDecision;

pub fn identity_mismatch_diagnostic(
    diagnostic_id: DiagnosticId,
    raw_event: &RawCollectorEvent,
) -> DiagnosticRecord {
    let mut diagnostic = DiagnosticRecord::new(
        diagnostic_id,
        None,
        DiagnosticKind::IdentityUnverified,
        DiagnosticSeverity::Warning,
        std::time::SystemTime::now(),
        "raw event could not be matched to an active trace",
    )
    .with_metadata("raw.collector", raw_event.envelope.collector.as_str())
    .with_metadata(
        "raw.observed_at_unix_nanos",
        system_time_unix_nanos(raw_event.envelope.observed_at),
    );
    add_payload_metadata(&mut diagnostic, &raw_event.payload);
    diagnostic
}

pub fn policy_diagnostic(
    diagnostic_id: DiagnosticId,
    trace_id: TraceId,
    process: &ProcessIdentity,
    decision: &PolicyDecision,
) -> Vec<DiagnosticRecord> {
    let maybe_kind = match decision.record.verdict {
        PolicyVerdict::Allow => None,
        PolicyVerdict::Redact => Some(DiagnosticKind::PolicyRedacted),
        PolicyVerdict::Drop => Some(DiagnosticKind::PolicyFiltered),
        PolicyVerdict::Fatal => Some(DiagnosticKind::RuntimeFatal),
    };

    maybe_kind
        .map(|kind| {
            vec![
                DiagnosticRecord::new(
                    diagnostic_id,
                    Some(trace_id),
                    kind,
                    DiagnosticSeverity::Info,
                    std::time::SystemTime::now(),
                    decision
                        .record
                        .note
                        .clone()
                        .unwrap_or_else(|| "policy decision applied".to_string()),
                )
                .with_process(process.clone()),
            ]
        })
        .unwrap_or_default()
}

fn add_payload_metadata(diagnostic: &mut DiagnosticRecord, payload: &RawObservationPayload) {
    match payload {
        RawObservationPayload::Process {
            operation,
            parent,
            metadata,
        } => {
            insert_metadata(diagnostic, "raw.payload_kind", "process");
            insert_metadata(diagnostic, "raw.operation", operation);
            if let Some(parent) = parent {
                add_observation_metadata(diagnostic, "raw.parent", parent);
            }
            copy_metadata_keys(
                diagnostic,
                metadata,
                &["ppid", "seccomp_observed", "exit_code"],
            );
        }
        RawObservationPayload::File {
            operation,
            metadata,
            ..
        } => {
            insert_metadata(diagnostic, "raw.payload_kind", "file");
            insert_metadata(diagnostic, "raw.operation", operation);
            copy_metadata_keys(diagnostic, metadata, &["syscall", "fd", "result"]);
        }
        RawObservationPayload::Net {
            transport,
            size,
            result,
            metadata,
            ..
        } => {
            insert_metadata(diagnostic, "raw.payload_kind", "net");
            insert_metadata(diagnostic, "raw.transport", transport);
            if let Some(size) = size {
                insert_metadata(diagnostic, "raw.size", size.to_string());
            }
            if let Some(result) = result {
                insert_metadata(diagnostic, "raw.result", result.to_string());
            }
            copy_metadata_keys(diagnostic, metadata, &["operation", "syscall_family", "fd"]);
        }
        RawObservationPayload::Ipc {
            channel, metadata, ..
        } => {
            insert_metadata(diagnostic, "raw.payload_kind", "ipc");
            insert_metadata(diagnostic, "raw.channel", channel);
            copy_metadata_keys(diagnostic, metadata, &["operation", "syscall", "result"]);
        }
        RawObservationPayload::Stdio {
            stream, metadata, ..
        } => {
            insert_metadata(diagnostic, "raw.payload_kind", "stdio");
            insert_metadata(diagnostic, "raw.stream", stream);
            copy_metadata_keys(diagnostic, metadata, &["fd", "result"]);
        }
    }
}

fn add_observation_metadata(
    diagnostic: &mut DiagnosticRecord,
    prefix: &str,
    observation: &model_core::process::ProcessObservation,
) {
    if let Some(host) = &observation.host {
        insert_metadata(diagnostic, format!("{prefix}.host_pid"), host.pid);
        insert_metadata(
            diagnostic,
            format!("{prefix}.start_ticks"),
            host.start_time_ticks,
        );
        if let Some(start_boottime_ns) = host.start_boottime_ns {
            insert_metadata(
                diagnostic,
                format!("{prefix}.start_boottime_ns"),
                start_boottime_ns,
            );
        }
    }
    if let Some(namespace) = &observation.namespace {
        insert_metadata(
            diagnostic,
            format!("{prefix}.pid_namespace"),
            namespace.pid_namespace.as_str(),
        );
        insert_metadata(diagnostic, format!("{prefix}.namespace_pid"), namespace.pid);
    }
}

fn copy_metadata_keys(
    diagnostic: &mut DiagnosticRecord,
    metadata: &std::collections::BTreeMap<String, String>,
    keys: &[&str],
) {
    for key in keys {
        if let Some(value) = metadata.get(*key) {
            insert_metadata(diagnostic, format!("raw.metadata.{key}"), value);
        }
    }
}

fn insert_metadata(
    diagnostic: &mut DiagnosticRecord,
    key: impl Into<String>,
    value: impl ToString,
) {
    diagnostic.metadata.insert(key.into(), value.to_string());
}

fn system_time_unix_nanos(value: SystemTime) -> String {
    value
        .duration_since(UNIX_EPOCH)
        .expect("raw event observed_at must be after Unix epoch")
        .as_nanos()
        .to_string()
}
