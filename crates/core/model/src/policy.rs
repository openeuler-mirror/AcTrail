//! Policy outcomes and redaction metadata shared across ingest and export.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyVerdict {
    Allow,
    Redact,
    Drop,
    Fatal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TruncationReason {
    PolicyLimit,
    TransportLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactionRecord {
    pub field: String,
    pub reason: String,
}

impl RedactionRecord {
    pub fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TruncationRecord {
    pub field: String,
    pub original_size: usize,
    pub retained_size: usize,
    pub reason: TruncationReason,
}

impl TruncationRecord {
    pub fn new(
        field: impl Into<String>,
        original_size: usize,
        retained_size: usize,
        reason: TruncationReason,
    ) -> Self {
        Self {
            field: field.into(),
            original_size,
            retained_size,
            reason,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyRecord {
    pub verdict: PolicyVerdict,
    pub redactions: Vec<RedactionRecord>,
    pub truncations: Vec<TruncationRecord>,
    pub note: Option<String>,
}

impl PolicyRecord {
    pub fn allow() -> Self {
        Self {
            verdict: PolicyVerdict::Allow,
            redactions: Vec::new(),
            truncations: Vec::new(),
            note: None,
        }
    }

    pub fn fatal(note: impl Into<String>) -> Self {
        Self {
            verdict: PolicyVerdict::Fatal,
            redactions: Vec::new(),
            truncations: Vec::new(),
            note: Some(note.into()),
        }
    }
}
