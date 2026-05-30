//! Policy decision result contracts.

use model_core::policy::{PolicyRecord, PolicyVerdict};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyDecision {
    pub record: PolicyRecord,
}

impl PolicyDecision {
    pub fn allow() -> Self {
        Self {
            record: PolicyRecord::allow(),
        }
    }

    pub fn drop(note: impl Into<String>) -> Self {
        Self {
            record: PolicyRecord {
                verdict: PolicyVerdict::Drop,
                redactions: Vec::new(),
                truncations: Vec::new(),
                note: Some(note.into()),
            },
        }
    }

    pub fn fatal(note: impl Into<String>) -> Self {
        Self {
            record: PolicyRecord::fatal(note),
        }
    }
}
