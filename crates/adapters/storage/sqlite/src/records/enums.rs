//! Enum-to-string encoders used in SQLite record storage.

use model_core::diagnostics::{DiagnosticKind, DiagnosticSeverity};
use model_core::event::EventKind;
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadRedactionState,
    PayloadSourceBoundary, PayloadTruncationState,
};
use model_core::policy::{PolicyVerdict, TruncationReason};
use model_core::process::{ExitObservationSource, MembershipState};
use model_core::trace::{TraceHealth, TraceLifecycleState};
use rusqlite::Error as SqlError;

pub fn encode_trace_lifecycle(value: TraceLifecycleState) -> &'static str {
    match value {
        TraceLifecycleState::Starting => "starting",
        TraceLifecycleState::Active => "active",
        TraceLifecycleState::Draining => "draining",
        TraceLifecycleState::Completed => "completed",
        TraceLifecycleState::Failed => "failed",
    }
}

pub fn decode_trace_lifecycle(raw: &str) -> Result<TraceLifecycleState, SqlError> {
    match raw {
        "starting" => Ok(TraceLifecycleState::Starting),
        "active" => Ok(TraceLifecycleState::Active),
        "draining" => Ok(TraceLifecycleState::Draining),
        "completed" => Ok(TraceLifecycleState::Completed),
        "failed" => Ok(TraceLifecycleState::Failed),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_trace_health(value: TraceHealth) -> &'static str {
    match value {
        TraceHealth::Clean => "clean",
        TraceHealth::Degraded => "degraded",
    }
}

pub fn decode_trace_health(raw: &str) -> Result<TraceHealth, SqlError> {
    match raw {
        "clean" => Ok(TraceHealth::Clean),
        "degraded" => Ok(TraceHealth::Degraded),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_membership_state(value: MembershipState) -> &'static str {
    match value {
        MembershipState::Starting => "starting",
        MembershipState::Active => "active",
        MembershipState::Exited => "exited",
        MembershipState::IdentityStale => "identity_stale",
    }
}

pub fn decode_membership_state(raw: &str) -> Result<MembershipState, SqlError> {
    match raw {
        "starting" => Ok(MembershipState::Starting),
        "active" => Ok(MembershipState::Active),
        "exited" => Ok(MembershipState::Exited),
        "identity_stale" => Ok(MembershipState::IdentityStale),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_exit_observation_source(value: ExitObservationSource) -> &'static str {
    match value {
        ExitObservationSource::Event => "event",
        ExitObservationSource::Reconciled => "reconciled",
    }
}

pub fn decode_exit_observation_source(raw: &str) -> Result<ExitObservationSource, SqlError> {
    match raw {
        "event" => Ok(ExitObservationSource::Event),
        "reconciled" => Ok(ExitObservationSource::Reconciled),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_event_kind(value: EventKind) -> &'static str {
    match value {
        EventKind::Process => "process",
        EventKind::File => "file",
        EventKind::Net => "net",
        EventKind::Ipc => "ipc",
        EventKind::Stdio => "stdio",
        EventKind::Application => "application",
        EventKind::Resource => "resource",
        EventKind::Control => "control",
        EventKind::Loss => "loss",
        EventKind::Label => "label",
        EventKind::Enforcement => "enforcement",
    }
}

pub fn decode_event_kind(raw: &str) -> Result<EventKind, SqlError> {
    match raw {
        "process" => Ok(EventKind::Process),
        "file" => Ok(EventKind::File),
        "net" => Ok(EventKind::Net),
        "ipc" => Ok(EventKind::Ipc),
        "stdio" => Ok(EventKind::Stdio),
        "application" => Ok(EventKind::Application),
        "resource" => Ok(EventKind::Resource),
        "control" => Ok(EventKind::Control),
        "loss" => Ok(EventKind::Loss),
        "label" => Ok(EventKind::Label),
        "enforcement" => Ok(EventKind::Enforcement),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_policy_verdict(value: PolicyVerdict) -> &'static str {
    match value {
        PolicyVerdict::Allow => "allow",
        PolicyVerdict::Redact => "redact",
        PolicyVerdict::Drop => "drop",
        PolicyVerdict::Fatal => "fatal",
    }
}

pub(crate) fn decode_policy_verdict(raw: &str) -> Result<PolicyVerdict, SqlError> {
    match raw {
        "allow" => Ok(PolicyVerdict::Allow),
        "redact" => Ok(PolicyVerdict::Redact),
        "drop" => Ok(PolicyVerdict::Drop),
        "fatal" => Ok(PolicyVerdict::Fatal),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_diagnostic_kind(value: DiagnosticKind) -> &'static str {
    match value {
        DiagnosticKind::CapabilityRejected => "capability_rejected",
        DiagnosticKind::OpportunisticUnbound => "opportunistic_unbound",
        DiagnosticKind::BootstrapPartial => "bootstrap_partial",
        DiagnosticKind::BootstrapGap => "bootstrap_gap",
        DiagnosticKind::IdentityUnverified => "identity_unverified",
        DiagnosticKind::IdentityMismatch => "identity_mismatch",
        DiagnosticKind::RuntimeDropped => "runtime_dropped",
        DiagnosticKind::RuntimeFatal => "runtime_fatal",
        DiagnosticKind::PolicyFiltered => "policy_filtered",
        DiagnosticKind::PolicyRedacted => "policy_redacted",
        DiagnosticKind::PolicyTruncated => "policy_truncated",
        DiagnosticKind::TracePurged => "trace_purged",
    }
}

pub fn decode_diagnostic_kind(raw: &str) -> Result<DiagnosticKind, SqlError> {
    match raw {
        "capability_rejected" => Ok(DiagnosticKind::CapabilityRejected),
        "opportunistic_unbound" => Ok(DiagnosticKind::OpportunisticUnbound),
        "bootstrap_partial" => Ok(DiagnosticKind::BootstrapPartial),
        "bootstrap_gap" => Ok(DiagnosticKind::BootstrapGap),
        "identity_unverified" => Ok(DiagnosticKind::IdentityUnverified),
        "identity_mismatch" => Ok(DiagnosticKind::IdentityMismatch),
        "runtime_dropped" => Ok(DiagnosticKind::RuntimeDropped),
        "runtime_fatal" => Ok(DiagnosticKind::RuntimeFatal),
        "policy_filtered" => Ok(DiagnosticKind::PolicyFiltered),
        "policy_redacted" => Ok(DiagnosticKind::PolicyRedacted),
        "policy_truncated" => Ok(DiagnosticKind::PolicyTruncated),
        "trace_purged" => Ok(DiagnosticKind::TracePurged),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_diagnostic_severity(value: DiagnosticSeverity) -> &'static str {
    match value {
        DiagnosticSeverity::Info => "info",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Error => "error",
    }
}

pub fn decode_diagnostic_severity(raw: &str) -> Result<DiagnosticSeverity, SqlError> {
    match raw {
        "info" => Ok(DiagnosticSeverity::Info),
        "warning" => Ok(DiagnosticSeverity::Warning),
        "error" => Ok(DiagnosticSeverity::Error),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub(crate) fn encode_truncation_reason(value: TruncationReason) -> &'static str {
    match value {
        TruncationReason::PolicyLimit => "policy_limit",
        TruncationReason::TransportLimit => "transport_limit",
    }
}

pub(crate) fn decode_truncation_reason(raw: &str) -> Result<TruncationReason, SqlError> {
    match raw {
        "policy_limit" => Ok(TruncationReason::PolicyLimit),
        "transport_limit" => Ok(TruncationReason::TransportLimit),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_payload_source_boundary(value: PayloadSourceBoundary) -> &'static str {
    match value {
        PayloadSourceBoundary::TlsUserSpace => "tls_user_space",
        PayloadSourceBoundary::Syscall => "syscall",
        PayloadSourceBoundary::Stdio => "stdio",
    }
}

pub fn decode_payload_source_boundary(raw: &str) -> Result<PayloadSourceBoundary, SqlError> {
    match raw {
        "tls_user_space" => Ok(PayloadSourceBoundary::TlsUserSpace),
        "syscall" => Ok(PayloadSourceBoundary::Syscall),
        "stdio" => Ok(PayloadSourceBoundary::Stdio),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_payload_content_state(value: PayloadContentState) -> &'static str {
    match value {
        PayloadContentState::Plaintext => "plaintext",
        PayloadContentState::Ciphertext => "ciphertext",
    }
}

pub fn decode_payload_content_state(raw: &str) -> Result<PayloadContentState, SqlError> {
    match raw {
        "plaintext" => Ok(PayloadContentState::Plaintext),
        "ciphertext" => Ok(PayloadContentState::Ciphertext),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_payload_direction(value: PayloadDirection) -> &'static str {
    match value {
        PayloadDirection::Outbound => "outbound",
        PayloadDirection::Inbound => "inbound",
    }
}

pub fn decode_payload_direction(raw: &str) -> Result<PayloadDirection, SqlError> {
    match raw {
        "outbound" => Ok(PayloadDirection::Outbound),
        "inbound" => Ok(PayloadDirection::Inbound),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_payload_redaction_state(value: PayloadRedactionState) -> &'static str {
    match value {
        PayloadRedactionState::NotRequired => "not_required",
        PayloadRedactionState::Redacted => "redacted",
        PayloadRedactionState::Unredacted => "unredacted",
    }
}

pub fn decode_payload_redaction_state(raw: &str) -> Result<PayloadRedactionState, SqlError> {
    match raw {
        "not_required" => Ok(PayloadRedactionState::NotRequired),
        "redacted" => Ok(PayloadRedactionState::Redacted),
        "unredacted" => Ok(PayloadRedactionState::Unredacted),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_payload_truncation_state(value: PayloadTruncationState) -> &'static str {
    match value {
        PayloadTruncationState::Complete => "complete",
        PayloadTruncationState::Truncated => "truncated",
    }
}

pub fn decode_payload_truncation_state(raw: &str) -> Result<PayloadTruncationState, SqlError> {
    match raw {
        "complete" => Ok(PayloadTruncationState::Complete),
        "truncated" => Ok(PayloadTruncationState::Truncated),
        _ => Err(SqlError::InvalidQuery),
    }
}

pub fn encode_payload_operation_completion_state(
    value: PayloadOperationCompletionState,
) -> &'static str {
    value.as_str()
}

pub fn decode_payload_operation_completion_state(
    raw: &str,
) -> Result<PayloadOperationCompletionState, SqlError> {
    match raw {
        "unknown" => Ok(PayloadOperationCompletionState::Unknown),
        "success" => Ok(PayloadOperationCompletionState::Success),
        "partial" => Ok(PayloadOperationCompletionState::Partial),
        "failed" => Ok(PayloadOperationCompletionState::Failed),
        _ => Err(SqlError::InvalidQuery),
    }
}
