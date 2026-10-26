//! Enum-to-string encoders used in SQLite record storage.

use model_core::diagnostics::{DiagnosticKind, DiagnosticSeverity};
use model_core::policy::TruncationReason;
use model_core::process::{ExitObservationSource, MembershipState};
use model_core::trace::{TraceHealth, TraceLifecycleState};
use rusqlite::Error as SqlError;

pub fn encode_trace_lifecycle(value: TraceLifecycleState) -> &'static str {
    value.as_storage_str()
}

pub fn decode_trace_lifecycle(raw: &str) -> Result<TraceLifecycleState, SqlError> {
    TraceLifecycleState::from_storage_str(raw).ok_or(SqlError::InvalidQuery)
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

pub fn encode_diagnostic_kind(value: DiagnosticKind) -> &'static str {
    match value {
        DiagnosticKind::CapabilityRejected => "capability_rejected",
        DiagnosticKind::OpportunisticUnbound => "opportunistic_unbound",
        DiagnosticKind::BootstrapPartial => "bootstrap_partial",
        DiagnosticKind::BootstrapGap => "bootstrap_gap",
        DiagnosticKind::IdentityUnverified => "identity_unverified",
        DiagnosticKind::IdentityMismatch => "identity_mismatch",
        DiagnosticKind::RuntimeDropped => "runtime_dropped",
        DiagnosticKind::RuntimeFailure => "runtime_failure",
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
        "runtime_failure" => Ok(DiagnosticKind::RuntimeFailure),
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
