//! Diagnostics emitted by capability negotiation, runtime, policy, and retention.

use std::collections::BTreeMap;
use std::time::SystemTime;

use crate::ids::{DiagnosticId, TraceId};
use crate::process::ProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    CapabilityRejected,
    OpportunisticUnbound,
    BootstrapPartial,
    BootstrapGap,
    IdentityUnverified,
    IdentityMismatch,
    RuntimeDropped,
    RuntimeFatal,
    PolicyFiltered,
    PolicyRedacted,
    PolicyTruncated,
    TracePurged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord {
    pub diagnostic_id: DiagnosticId,
    pub trace_id: Option<TraceId>,
    pub process: Option<ProcessIdentity>,
    pub kind: DiagnosticKind,
    pub severity: DiagnosticSeverity,
    pub emitted_at: SystemTime,
    pub message: String,
    pub metadata: BTreeMap<String, String>,
}

impl DiagnosticRecord {
    pub fn new(
        diagnostic_id: DiagnosticId,
        trace_id: Option<TraceId>,
        kind: DiagnosticKind,
        severity: DiagnosticSeverity,
        emitted_at: SystemTime,
        message: impl Into<String>,
    ) -> Self {
        Self {
            diagnostic_id,
            trace_id,
            process: None,
            kind,
            severity,
            emitted_at,
            message: message.into(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_process(mut self, process: ProcessIdentity) -> Self {
        self.process = Some(process);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}
