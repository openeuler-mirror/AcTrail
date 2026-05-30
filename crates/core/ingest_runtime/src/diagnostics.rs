//! Ingest-time diagnostic shaping and escalation boundaries.

use model_core::diagnostics::{DiagnosticKind, DiagnosticRecord, DiagnosticSeverity};
use model_core::ids::{DiagnosticId, TraceId};
use model_core::policy::PolicyVerdict;
use model_core::process::ProcessIdentity;
use policy_evaluate_contract::decision::PolicyDecision;

pub fn identity_mismatch_diagnostic(
    diagnostic_id: DiagnosticId,
    process: ProcessIdentity,
) -> DiagnosticRecord {
    DiagnosticRecord::new(
        diagnostic_id,
        None,
        DiagnosticKind::IdentityUnverified,
        DiagnosticSeverity::Warning,
        std::time::SystemTime::now(),
        "raw event could not be matched to an active trace",
    )
    .with_process(process)
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
